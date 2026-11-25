import init, { cancel_worker_request, run_worker_request } from "./rynthv.js";
import { setActive } from "./worker-bridge.js";

const ready = init();
const queue = [];
let active;
const maxPending = 12;

async function drain() {
    if (active || !queue.length) return;
    active = queue.shift();
    const { id, request } = active;
    setActive(id);
    try {
        await ready;
        const result = await run_worker_request(request);
        self.postMessage({ id, result });
    } catch (error) {
        self.postMessage({ id, error: error?.message ?? String(error) });
    } finally {
        active = undefined;
        void drain();
    }
}

self.onmessage = ({ data }) => {
    if (data.cancel !== undefined) {
        if (active?.id === data.cancel) cancel_worker_request();
        const index = queue.findIndex(item => item.id === data.cancel);
        if (index >= 0) {
            const [{ id }] = queue.splice(index, 1);
            self.postMessage({ id, error: "CANCELLED" });
        }
        return;
    }
    if (queue.length >= maxPending) {
        self.postMessage({ id: data.id, error: "浏览器合成队列已满，请等待当前任务完成。" });
        return;
    }
    queue.push(data);
    void drain();
};
