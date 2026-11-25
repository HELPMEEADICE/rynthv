let acknowledge;

export function cpu_progress(message, pcm) {
    return new Promise(resolve => {
        acknowledge = resolve;
        self.postMessage({ done: false, packet: [message, pcm] }, pcm ? [pcm.buffer] : []);
    });
}

export function acknowledgeProgress() {
    const resolve = acknowledge;
    acknowledge = undefined;
    resolve?.();
}
