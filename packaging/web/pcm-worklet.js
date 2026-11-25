const SOURCE_RATE = 48_000;
const BLOCK = 24_000;
const MAX_FRAMES = 33_554_432;

class PcmPlayer extends AudioWorkletProcessor {
    constructor() {
        super();
        this.blocks = new Map();
        this.bufferedFrames = 0;
        this.step = SOURCE_RATE / sampleRate;
        this.position = 0;
        this.playing = false;
        this.loopStart = 0;
        this.loopEnd = 0;
        this.duration = Infinity;
        this.generation = 0;
        this.transport = 0;
        this.report = 0;
        this.clicks = undefined;
        this.beat = 0;
        this.occupied = undefined;
        this.port.onmessage = ({ data }) => this.message(data);
    }

    message(data) {
        switch (data.kind) {
            case "reset":
                this.generation = data.generation;
                this.blocks.clear();
                this.bufferedFrames = 0;
                this.duration = Infinity;
                this.occupied = undefined;
                break;
            case "layout":
                this.occupied = data.occupied;
                this.duration = data.frames;
                break;
            case "block": {
                const pcm = data.pcm ?? null;
                const frames = this.bufferedFrames + (pcm?.length ?? 0) / 2
                    - (this.blocks.get(data.index)?.length ?? 0) / 2;
                if (frames > MAX_FRAMES) {
                    this.port.postMessage({ generation: this.generation, error: "预览音频超过浏览器缓冲上限，请导出 WAV 播放。" });
                    break;
                }
                this.bufferedFrames = frames;
                this.blocks.set(data.index, pcm);
                break;
            }
            case "end":
                this.duration = Math.round(data.duration * SOURCE_RATE);
                break;
            case "transport":
                this.transport = data.transport;
                this.playing = data.playing;
                this.position = Math.max(0, data.seconds * SOURCE_RATE);
                this.report = 0;
                this.locateBeat();
                break;
            case "loop":
                this.loopStart = data.start * SOURCE_RATE;
                this.loopEnd = data.end * SOURCE_RATE;
                break;
            case "clicks":
                this.clicks = data;
                this.locateBeat();
                break;
        }
    }

    block(index) {
        const stored = this.blocks.get(index);
        if (stored !== undefined) return stored;
        const occupied = this.occupied;
        if (!occupied) return undefined;
        let first = 0;
        let end = occupied.length / 2;
        while (first < end) {
            const mid = (first + end) >>> 1;
            if (occupied[mid * 2 + 1] <= index) first = mid + 1;
            else end = mid;
        }
        return occupied[first * 2] <= index ? undefined : null;
    }

    locateBeat() {
        const times = this.clicks?.times;
        if (!times?.length) return;
        let first = 0;
        let end = times.length;
        while (first < end) {
            const mid = (first + end) >>> 1;
            if (Math.abs(times[mid]) * SOURCE_RATE <= this.position) first = mid + 1;
            else end = mid;
        }
        this.beat = Math.max(0, first - 1);
    }

    click() {
        const clicks = this.clicks;
        if (!clicks?.times.length) return 0;
        while (this.beat + 1 < clicks.times.length && Math.abs(clicks.times[this.beat + 1]) * SOURCE_RATE <= this.position) this.beat++;
        const beat = clicks.times[this.beat];
        const sound = beat < 0 || Object.is(beat, -0) ? clicks.strong : clicks.weak;
        const frame = Math.floor(this.position - Math.abs(beat) * SOURCE_RATE);
        return sound[frame] ?? 0;
    }

    process(inputs, outputs) {
        const [left, right] = outputs[0];
        const times = this.clicks?.times;
        const lastClick = times?.length ? (Math.abs(times[times.length - 1]) + 0.04) * SOURCE_RATE : 0;
        const end = Math.max(this.duration, lastClick);
        let active = this.playing;
        let blockIndex = -1;
        let block;
        if (active) {
            for (let index = 0; index < left.length; index++) {
                if (this.loopEnd > this.loopStart && this.position >= this.loopEnd) {
                    this.position = this.loopStart + (this.position - this.loopEnd) % (this.loopEnd - this.loopStart);
                    this.locateBeat();
                }
                if (this.position >= end && this.loopEnd <= this.loopStart) {
                    active = false;
                    break;
                }
                const frame = Math.floor(this.position);
                const current = Math.floor(frame / BLOCK);
                if (current !== blockIndex) {
                    blockIndex = current;
                    block = this.block(current);
                }
                if (block === undefined && frame < this.duration) break;
                const click = this.click();
                if (!block || frame >= this.duration) {
                    left[index] = right[index] = click;
                    this.position += this.step;
                    continue;
                }
                const sample = (frame % BLOCK) * 2;
                const a = block[sample];
                const b = block[sample + 1];
                const fraction = this.position - frame;
                if (sample + 2 < block.length) {
                    left[index] = a + fraction * (block[sample + 2] - a) + click;
                    right[index] = b + fraction * (block[sample + 3] - b) + click;
                } else {
                    const next = this.block(current + 1);
                    const nextLeft = next === null ? 0 : next?.[0] ?? a;
                    const nextRight = next === null ? 0 : next?.[1] ?? b;
                    left[index] = a + fraction * (nextLeft - a) + click;
                    right[index] = b + fraction * (nextRight - b) + click;
                }
                this.position += this.step;
            }
        }
        if (this.report++ % 6 === 0) {
            this.port.postMessage({
                generation: this.generation, transport: this.transport, active, seconds: this.position / SOURCE_RATE,
            });
        }
        return true;
    }
}

registerProcessor("rynthv-pcm", PcmPlayer);
