# SynthV Flat in RynthV

RynthV loads supported plaintext Flat voice packages and processes their notes, lyrics, and voice conditions through its Rust duration, F0, acoustic, and vocoder stages. The editor uses its existing synthesis workers and WAV export. Flat models use the native DDS path and require no SV2 model identity.

## Install and use a voice

1. Open **管理歌声** (Manage Voices) and select **导入 Flat…**. Choose one or more `.sfpk` or `.sfpks` files.
2. Open **详情与诊断** for the voice. **Flat 模型依赖** lists the acoustic, vocoder, duration, and F0 model IDs and their available sources. Set **SynthV Flat Base Model 目录** to resolve external models, then select **刷新**.
3. Set **发音词典目录** to the existing `clf-data` resources and select the voice for a singing track. Enter lyrics or explicit phonemes, edit notes, and export through the project's WAV workflow.
4. Change the timbre weights under **唱法** in the voice panel, or expand **调整当前唱法** in the selected voice's details. The same controls accept multiple simultaneous modes.

**SynthV Flat 声库目录** scans top-level SFPKs and RynthV's managed imports. The default is the `flat-voices` folder in RynthV's configuration directory. Batch import stages the individual packages and publishes the batch together. **移除 Flat 导入副本** removes the managed copy. The source package and shared model directory remain outside this operation. The **仅显示 SynthV Flat** filter restricts the voice list to Flat packages.

Model files may be bundled at `models/<id>.dnni`, supplied by another scanned SFPK, or installed at `<model-dir>/<id>.dnni` or `<model-dir>/<role>/<id>.dnni`. Use the identifiers in the voice manifest as filenames. The role directories for native stages are `acoustic`, `vocoder`, `duration`, and `f0`.

## Headless workflow

```sh
rynthv-synth inspect-flat --input voice.sfpk
rynthv-synth inspect-flat --input voices.sfpks
rynthv-synth import-flat --input voices.sfpks --flat-dir ./flat-voices
rynthv-synth list-flat-models --flat-dir ./flat-voices --model-dir ./flat-models
rynthv-synth validate-flat --input voice.sfpk --model-dir ./flat-models
rynthv-synth render-score --voice voice.sfpk --model-dir ./flat-models \
  --input examples/synthesis/mandarin-phrase.json --wav voice.wav
```

Use the imported SFPK path returned in the JSON report for an individual voice from a batch. `inspect-flat` reports the manifest and available sources. `validate-flat` prepares a single voice with the same loader used by the editor and returns errors for missing resources or incompatible stage structures. A validation pass does not render a score. Exit code `0` indicates success, `1` indicates an operation error, and `2` indicates invalid Flat-command arguments. Use `--output REPORT.json` to write the report to a file.

To blend two Flat timbres, add `--blend-voice second.sfpk --blend-weight 0.35` to `render-score`. Both voices must resolve to identical acoustic model bytes. The blend interpolates the base and default acoustic conditions. Pitch, timing, pronunciation, selectable Vocal Modes, and the vocoder come from `--voice`.

See [the format notes](synthv-flat-format.md), [the compatibility and verification report](synthv-flat-compatibility.md), and [native WebGPU inference](webgpu-inference.md).
