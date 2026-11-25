let active;

export function setActive(id) { active = id; }

export function emit_event(message, pcm) {
    self.postMessage({ id: active, event: message, pcm }, pcm ? [pcm.buffer] : []);
}

export function yield_worker() {
    return new Promise(resolve => setTimeout(resolve, 0));
}
