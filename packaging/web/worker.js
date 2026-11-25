import init, { cancel_worker_request, prioritize_worker_preview, run_worker_request } from "./rynthv.js";
import { setActive } from "./worker-bridge.js";

let ready;
const queue = [];
let active;
const maxPending = 12;

async function drain() {
    if (active || !queue.length) return;
    active = queue.shift();
    const { id, request, writes } = active;
    setActive(id);
    try {
        await ready;
        const run = () => {
            if (active.cancelled) throw new Error("CANCELLED");
            active.started = true;
            return run_worker_request(request, active.seconds);
        };
        const result = await (navigator.locks
            ? navigator.locks.request("rynthv-resources", { mode: writes ? "exclusive" : "shared" }, run)
            : run());
        self.postMessage({ id, result });
    } catch (error) {
        self.postMessage({ id, error: error?.message ?? String(error) });
    } finally {
        active = undefined;
        void drain();
    }
}

self.onmessage = ({ data }) => {
    if (data.module) {
        ready = init({ module_or_path: data.module });
        void ready.catch(() => {});
        return;
    }
    if (data.demand !== undefined) {
        if (active?.id === data.demand) {
            active.seconds = data.seconds;
            if (active.started) prioritize_worker_preview(data.seconds);
        } else {
            const queued = queue.find(item => item.id === data.demand);
            if (queued) queued.seconds = data.seconds;
        }
        return;
    }
    if (data.cancel !== undefined) {
        if (active?.id === data.cancel) {
            active.cancelled = true;
            if (active.started) cancel_worker_request();
        }
        const index = queue.findIndex(item => item.id === data.cancel);
        if (index >= 0) {
            const [{ id }] = queue.splice(index, 1);
            self.postMessage({ id, error: "CANCELLED" });
        }
        return;
    }
    if (queue.length >= maxPending && !data.preview) {
        self.postMessage({ id: data.id, error: "浏览器合成队列已满，请等待当前任务完成。" });
        return;
    }
    if (data.preview) {
        // Keep resource writes ordered while moving previews ahead of queued readers.
        const index = queue.findLastIndex(item => item.writes || item.preview) + 1;
        queue.splice(index, 0, data);
    } else {
        queue.push(data);
    }
    void drain();
};
