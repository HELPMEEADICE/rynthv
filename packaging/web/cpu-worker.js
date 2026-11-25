import init, { run_cpu_job, cancel_cpu_job, prioritize_cpu_job } from "./rynthv.js";
import { acknowledgeProgress } from "./cpu-worker-bridge.js";
import { restrictToReads } from "./opfs.js";

restrictToReads();
let ready;
let active;

async function render(entry) {
    try {
        await ready;
        if (entry.cancelled) throw new Error("CANCELLED");
        entry.started = true;
        const packet = await run_cpu_job(entry.job, "seconds" in entry, entry.seconds);
        self.postMessage({ done: true, packet }, [packet[1].buffer, packet[2].buffer]);
    } catch (error) {
        const fatal = (!entry.started && !entry.cancelled) || error instanceof WebAssembly.RuntimeError;
        self.postMessage({ done: true, fatal, error: error?.message ?? String(error) });
    } finally {
        active = undefined;
    }
}

self.onmessage = ({ data }) => {
    if (data.module) {
        ready = init({ module_or_path: data.module });
        void ready.catch(() => {});
    } else if (data.job !== undefined) {
        active = { job: data.job };
        void render(active);
    } else if (data.ack) {
        acknowledgeProgress();
    } else if (data.cancel && active) {
        active.cancelled = true;
        if (active.started) cancel_cpu_job();
    } else if (active) {
        active.seconds = data.seconds;
        if (active.started) prioritize_cpu_job(data.seconds);
    }
};
