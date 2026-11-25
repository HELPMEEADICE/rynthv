let context;
let output;
let loading;
let error = "";
let clock;
let lastCommand = { kind: "transport", playing: false, seconds: 0 };
let bounds = { kind: "loop", start: 0, end: 0 };
let generation = 0;
let transport = 0;
let duration;
const queued = new Map();
let queuedFrames = 0;
let layout;
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
                else if (data.transport === transport) clock = data.active ? data.seconds : undefined;
            };
            output.port.postMessage({ kind: "reset", generation });
            if (layout) output.port.postMessage(layout);
            for (const part of queued.values()) postBlock(part);
            queued.clear();
            queuedFrames = 0;
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
    clock = undefined;
    layout = undefined;
    queued.clear();
    queuedFrames = 0;
    output?.port.postMessage({ kind: "reset", generation });
}

function postBlock(part) {
    output.port.postMessage(part, part.pcm ? [part.pcm.buffer] : []);
}

export function audio_block(index, pcm) {
    const part = { kind: "block", index, pcm };
    if (output) {
        postBlock(part);
    } else {
        const nextFrames = queuedFrames + (pcm?.length ?? 0) / 2
            - (queued.get(index)?.pcm?.length ?? 0) / 2;
        if (nextFrames <= 33_554_432) {
            queuedFrames = nextFrames;
            queued.set(index, part);
        } else {
            error = "预览音频超过浏览器缓冲上限，请导出 WAV 播放。";
        }
    }
}

export function audio_layout(frames, occupied) {
    layout = { kind: "layout", frames, occupied };
    output?.port.postMessage(layout);
}

export function audio_end(seconds) {
    duration = seconds;
    output?.port.postMessage({ kind: "end", duration });
}

export function audio_transport(playing, seconds) {
    lastCommand = { kind: "transport", playing, seconds, transport: ++transport };
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
