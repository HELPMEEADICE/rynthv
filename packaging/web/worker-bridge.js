let active;
let yields;
let resume;

export function setActive(id) { active = id; }

export function emit_event(message, pcm) {
    self.postMessage({ id: active, event: message, pcm }, pcm ? [pcm.buffer] : []);
}

export function yield_worker() {
    if (!yields) {
        yields = new MessageChannel();
        yields.port1.onmessage = () => {
            const next = resume;
            resume = undefined;
            next();
        };
    }
    return new Promise(resolve => {
        resume = resolve;
        yields.port2.postMessage(null);
    });
}
