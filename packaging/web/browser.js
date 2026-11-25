let worker;
let nextRequest = 1;
const requests = new Map();

function stopWorker(error) {
    worker?.terminate();
    worker = undefined;
    for (const task of requests.values()) task.reject(error);
    requests.clear();
}

function workerInstance() {
    if (!worker) {
        worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
        worker.onmessage = ({ data }) => {
            const request = requests.get(data.id);
            if (!request) return;
            if (data.event) {
                request.listener(data.event, data.pcm);
            } else {
                requests.delete(data.id);
                if (data.error) request.reject(new Error(data.error));
                else request.resolve(data.result);
            }
        };
        worker.onerror = event => stopWorker(new Error(event.message || "合成 Worker 已停止"));
        worker.onmessageerror = () => stopWorker(new Error("无法读取合成 Worker 消息"));
    }
    return worker;
}

export function start_request(request, writes, listener) {
    const id = nextRequest++;
    const promise = new Promise((resolve, reject) => {
        try {
            const instance = workerInstance();
            requests.set(id, { resolve, reject, listener });
            instance.postMessage({ id, request, writes });
        } catch (error) {
            requests.delete(id);
            reject(error);
        }
    });
    return [id, promise];
}

export function cancel_request(id) { worker?.postMessage({ cancel: id }); }

export function set_title(title) { document.title = title; }

export function storage_usage() {
    return navigator.storage.estimate().then(({ usage, quota }) => JSON.stringify({ usage, quota }));
}
