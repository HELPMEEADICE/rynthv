let wasmModule;
let wake;
let notified = false;
let stopping;
const slots = new Map();
const inbox = [];

export function configureCpuPool(module) { wasmModule = module; }

export function cpu_slots() {
    const memory = navigator.deviceMemory ?? Infinity;
    const memoryLimit = memory <= 2 ? 1 : memory <= 4 ? 2 : 3;
    return Math.min(memoryLimit, Math.max(1, (navigator.hardwareConcurrency || 2) - 1));
}

export function wakeCpuPool() {
    notified = true;
    wake?.();
    wake = undefined;
}

export function wait_cpu_packets() {
    if (inbox.length || notified) {
        notified = false;
        return Promise.resolve();
    }
    return new Promise(resolve => {
        wake = () => {
            notified = false;
            resolve();
        };
    });
}

function receive(slot, entry, { done, error, packet }) {
    if (slots.get(slot) !== entry || !entry.busy) return;
    if (done) entry.busy = false;
    if (stopping) {
        if (!done) entry.worker.postMessage({ ack: true });
        if ([...slots.values()].every(item => !item.busy)) {
            const resolve = stopping;
            stopping = undefined;
            resolve();
        }
    } else {
        inbox.push([slot, done, error, packet]);
        wakeCpuPool();
    }
}

function fail(slot, entry, error) {
    if (slots.get(slot) !== entry) return;
    entry.worker?.terminate();
    receive(slot, entry, { done: true, error });
    slots.delete(slot);
}

export function start_cpu_job(slot, job) {
    let entry = slots.get(slot);
    try {
        if (!entry) {
            entry = { busy: true };
            slots.set(slot, entry);
            const worker = new Worker(new URL("./cpu-worker.js", import.meta.url), { type: "module" });
            entry.worker = worker;
            worker.onmessage = ({ data }) => {
                if (data.fatal) fail(slot, entry, data.error);
                else receive(slot, entry, data);
            };
            worker.onerror = event => {
                event.preventDefault();
                fail(slot, entry, event.message || "CPU Worker 已停止");
            };
            worker.onmessageerror = () => fail(slot, entry, "无法读取 CPU Worker 消息");
            worker.postMessage({ module: wasmModule });
        }
        entry.busy = true;
        entry.worker.postMessage({ job });
    } catch (error) {
        fail(slot, entry, error?.message ?? String(error));
    }
}

export function poll_cpu_packet() {
    const message = inbox.shift();
    if (message && !message[1]) slots.get(message[0])?.worker.postMessage({ ack: true });
    return message;
}

export function prioritize_cpu_job(slot, seconds) {
    const entry = slots.get(slot);
    if (entry?.busy) entry.worker.postMessage({ seconds });
}

export function cancel_cpu_job(slot) {
    const entry = slots.get(slot);
    if (entry?.busy) entry.worker.postMessage({ cancel: true });
}

export function stop_cpu_jobs() {
    for (const slot of slots.keys()) cancel_cpu_job(slot);
    for (const [slot, done] of inbox.splice(0)) {
        if (!done) slots.get(slot)?.worker.postMessage({ ack: true });
    }
    if ([...slots.values()].every(entry => !entry.busy)) return Promise.resolve();
    return new Promise(resolve => {
        stopping = resolve;
    });
}

export function resetCpuPool() {
    for (const entry of slots.values()) entry.worker.terminate();
    slots.clear();
}
