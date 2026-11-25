let context;
let output;
let loading;
let error = "";
let clock;
let lastCommand = { kind: "transport", playing: false, seconds: 0 };
let bounds = { kind: "loop", start: 0, end: 0 };
let generation = 0;
let duration;
let firstFinal = true;
const queued = [];
let piano;
let clicks;

async function resume() {
    try {
        context ??= new AudioContext({ latencyHint: "interactive" });
        await context.resume();
        loading ??= context.audioWorklet.addModule(new URL("./pcm-worklet.js", import.meta.url)).then(() => {
            output = new AudioWorkletNode(context, "rynthv-pcm", { outputChannelCount: [2] });
            output.connect(context.destination);
            output.port.onmessage = ({ data }) => {
                if (data.generation !== generation) return;
                if (data.error) error = data.error;
                else clock = data.active ? data.seconds : undefined;
            };
            output.port.postMessage({ kind: "reset", generation });
            for (const part of queued.splice(0)) output.port.postMessage(part, [part.pcm.buffer]);
            output.port.postMessage(bounds);
            output.port.postMessage(lastCommand);
            if (clicks) output.port.postMessage(clicks);
            if (duration !== undefined) output.port.postMessage({ kind: "end", duration });
        }).catch(cause => { loading = undefined; throw cause; });
        await loading;
        error = "";
    } catch (cause) {
        error = cause?.message ?? String(cause);
    }
}

export function install_audio(canvas) {
    canvas.addEventListener("pointerdown", () => void resume(), { capture: true });
    window.addEventListener("keydown", () => void resume(), { capture: true });
}

export function audio_reset() {
    generation++;
    duration = undefined;
    firstFinal = true;
    clock = undefined;
    queued.length = 0;
    output?.port.postMessage({ kind: "reset", generation });
}

export function audio_pcm(seconds, pcm, finalMix) {
    const replace = finalMix && firstFinal;
    if (replace) {
        firstFinal = false;
        queued.length = 0;
    }
    const part = { kind: "pcm", start: Math.round(seconds * 48_000), pcm, finalMix, replace };
    if (output) output.port.postMessage(part, [pcm.buffer]);
    else queued.push(part);
}

export function audio_end(seconds) {
    duration = seconds;
    output?.port.postMessage({ kind: "end", duration });
}

export function audio_transport(playing, seconds) {
    lastCommand = { kind: "transport", playing, seconds };
    clock = playing ? seconds : undefined;
    output?.port.postMessage(lastCommand);
}

export function audio_loop(start, end) {
    bounds = { kind: "loop", start, end };
    output?.port.postMessage(bounds);
}

export function audio_clock() { return clock; }
export function audio_error() { return error || undefined; }

export function audio_clicks(times, strong, weak) {
    clicks = { kind: "clicks", times, strong, weak };
    output?.port.postMessage(clicks);
}

export function audio_note(samples) {
    if (!context) return;
    if (piano) { piano.stop(); piano = undefined; }
    if (!samples) return;
    const buffer = context.createBuffer(1, samples.length, 48_000);
    buffer.copyToChannel(samples, 0);
    const node = context.createBufferSource();
    node.buffer = buffer;
    node.connect(context.destination);
    node.onended = () => { if (piano === node) piano = undefined; };
    node.start();
    piano = node;
}
