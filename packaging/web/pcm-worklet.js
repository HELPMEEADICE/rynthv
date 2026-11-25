const SOURCE_RATE = 48_000;
const BLOCK = 4096;
const MAX_BLOCKS = 8192;

class PcmPlayer extends AudioWorkletProcessor {
    constructor() {
        super();
        this.blocks = new Map();
        this.step = SOURCE_RATE / sampleRate;
        this.position = 0;
        this.playing = false;
        this.loopStart = 0;
        this.loopEnd = 0;
        this.duration = Infinity;
        this.generation = 0;
        this.report = 0;
        this.clicks = undefined;
        this.beat = 0;
        this.schedule = undefined;
        this.boundary = 0;
        this.finalMix = false;
        this.port.onmessage = ({ data }) => this.message(data);
    }

    message(data) {
        switch (data.kind) {
            case "reset":
                this.generation = data.generation;
                this.blocks.clear();
                this.duration = Infinity;
                this.schedule = undefined;
                this.finalMix = false;
                break;
            case "pcm":
                if (data.replace) this.blocks.clear();
                this.finalMix = data.finalMix || this.finalMix;
                this.append(data);
                break;
            case "end":
                this.duration = data.duration * SOURCE_RATE;
                break;
            case "transport":
                this.playing = data.playing;
                this.position = Math.max(0, data.seconds * SOURCE_RATE);
                this.report = 0;
                this.locateBeat();
                this.locateBoundary();
                break;
            case "loop":
                this.loopStart = data.start * SOURCE_RATE;
                this.loopEnd = data.end * SOURCE_RATE;
                break;
            case "clicks":
                this.clicks = data;
                this.locateBeat();
                break;
            case "schedule":
                this.schedule = data;
                this.locateBoundary();
                break;
        }
    }

    locateBoundary() {
        const points = this.schedule?.boundaries;
        this.boundary = 0;
        if (!points?.length) return;
        let end = points.length;
        while (this.boundary < end) {
            const mid = (this.boundary + end) >>> 1;
            if (points[mid] <= this.position) this.boundary = mid + 1;
            else end = mid;
        }
    }

    expected(frame) {
        const schedule = this.schedule;
        if (this.finalMix || this.duration !== Infinity) return 0;
        if (!schedule || schedule.waitForMix || !schedule.boundaries.length) return Infinity;
        while (this.boundary < schedule.boundaries.length && schedule.boundaries[this.boundary] <= frame) this.boundary++;
        if (this.boundary === schedule.boundaries.length) return Infinity;
        return this.boundary ? schedule.counts[this.boundary - 1] : 0;
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

    append({ start, pcm, finalMix }) {
        for (let offset = 0; offset < pcm.length / 2;) {
            const frame = start + offset;
            const index = Math.floor(frame / BLOCK);
            const from = frame % BLOCK;
            const length = Math.min(BLOCK - from, pcm.length / 2 - offset);
            let block = this.blocks.get(index);
            if (!block) {
                if (this.blocks.size >= MAX_BLOCKS) {
                    this.port.postMessage({ generation: this.generation, error: "预览音频超过浏览器缓冲上限，请导出 WAV 播放。" });
                    return;
                }
                block = { samples: new Float32Array(BLOCK * 2), ready: new Uint16Array(BLOCK) };
                this.blocks.set(index, block);
            }
            if (finalMix) {
                block.samples.set(pcm.subarray(offset * 2, (offset + length) * 2), from * 2);
                block.ready.fill(1, from, from + length);
            } else {
                for (let sample = 0; sample < length * 2; sample++) {
                    block.samples[from * 2 + sample] += pcm[offset * 2 + sample];
                }
                for (let frame = from; frame < from + length; frame++) block.ready[frame]++;
            }
            offset += length;
        }
    }

    process(inputs, outputs) {
        const [left, right] = outputs[0];
        let active = this.playing;
        if (active) {
            for (let index = 0; index < left.length; index++) {
                if (this.loopEnd > this.loopStart && this.position >= this.loopEnd) {
                    this.position = this.loopStart + (this.position - this.loopEnd) % (this.loopEnd - this.loopStart);
                    this.locateBeat();
                    this.locateBoundary();
                }
                const times = this.clicks?.times;
                const lastClick = times?.length ? (Math.abs(times[times.length - 1]) + 0.04) * SOURCE_RATE : 0;
                if (this.position >= Math.max(this.duration, lastClick) && this.loopEnd <= this.loopStart) {
                    active = false;
                    break;
                }
                const frame = Math.floor(this.position);
                const blockIndex = Math.floor(frame / BLOCK);
                const offset = frame % BLOCK;
                const block = this.blocks.get(blockIndex);
                if ((block?.ready[offset] ?? 0) < this.expected(frame)) break;
                if (!block?.ready[offset]) {
                    if (this.finalMix && this.duration === Infinity) break;
                    left[index] = right[index] = this.click();
                    this.position += this.step;
                    continue;
                }
                let next = offset + 1;
                const successor = next < BLOCK ? block : this.blocks.get(blockIndex + 1);
                next %= BLOCK;
                const fraction = this.position - frame;
                const a = block.samples[offset * 2];
                const b = block.samples[offset * 2 + 1];
                const click = this.click();
                left[index] = a + fraction * ((successor?.samples[next * 2] ?? a) - a) + click;
                right[index] = b + fraction * ((successor?.samples[next * 2 + 1] ?? b) - b) + click;
                this.position += this.step;
            }
        }
        if (this.report++ % 6 === 0) {
            this.port.postMessage({ generation: this.generation, active, seconds: this.position / SOURCE_RATE });
        }
        return true;
    }
}

registerProcessor("rynthv-pcm", PcmPlayer);
