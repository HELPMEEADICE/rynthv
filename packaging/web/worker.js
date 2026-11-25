import init, { cancel_worker_request, run_worker_request } from "./rynthv.js";
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
        await (ready ??= init());
        const run = () => {
            if (active.cancelled) throw new Error("CANCELLED");
            active.started = true;
            return run_worker_request(request);
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
    if (queue.length >= maxPending) {
        self.postMessage({ id: data.id, error: "浏览器合成队列已满，请等待当前任务完成。" });
        return;
    }
    queue.push(data);
    void drain();
};
