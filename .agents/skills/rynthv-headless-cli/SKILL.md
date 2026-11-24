---
name: rynthv-headless-cli
description: >-
  RynthV Headless CLI (`rynthv -cli`) 操作与自动化集成指南。当需要通过命令行、JSON Lines
  (`--stdio`) 或批量事务 (`project apply` / `batch run`) 在无桌面环境下创建、检查、编辑、
  调音、分析或渲染 `.svp` 歌声合成工程，或配置声库、词典、模型上下文与 CPU/WebGPU 计算后端时使用。
  Use when automating RynthV via `rynthv -cli`, `--stdio`, SVP editing, vocal
  synthesis rendering, or pitch/audio analysis.
---

# RynthV Headless CLI 对接指南

通过 `rynthv -cli` 在无 GUI / 无桌面环境中完成 SVP 工程构建、MIDI/音频导入、歌词音素解析、参数与音高调音、离线渲染与声学分析闭环。

> 如需完整命令参数表、参数范围与返回结构，按需查阅 [命令参考](./references/command-reference.md) 与 [协议与数据模式](./references/protocols-and-schemas.md)；持久化会话优先复用 [rynthv_stdio.py](./scripts/rynthv_stdio.py)。

## 1. 入口与运行前提

- **强制前缀**：第一个参数必须是 `-cli` 或 `--cli`（如 `rynthv -cli ...`）。不带该前缀会尝试启动 GUI 窗口（纯 CLI 构建会返回退出码 `2`）。
- **自动化标准参数**：单次命令调用始终带上 `--json --quiet --non-interactive`：
  - `--json`：stdout 只输出单行 UTF-8 JSON 结果信封（`schema_version: 1`）。
  - `--quiet`：抑制渲染进度与人类可读日志输出，避免污染解析。
  - `--non-interactive`：阻止无子命令时隐式进入交互式 REPL。
- **纯无头服务端构建**：若运行环境无 X11/Wayland/音频设备，使用纯 CLI 特性编译：
  ```sh
  cargo build --release --locked --no-default-features --features cli --bin rynthv
  ```
- **低层诊断边界**：`rynthv-synth` 仅用于脱离工程的底层模型诊断（`render-score`、`run-module`、`resolve-phone`、`list-compute-devices --validate`）或 Windows x64 从运行进程/会话文件导入 SV2 上下文（`import-context --process-id / --session-file / --identity`）；常规工程制作一律使用 `rynthv -cli`。

## 2. 首轮环境预检 (Preflight)

首次接入环境时，按顺序探明可用资源与后端能力，禁止盲猜声库 ID 或计算后端：

```sh
rynthv -cli doctor --json --quiet --non-interactive
rynthv -cli config show --json --quiet --non-interactive
rynthv -cli voice list --json --quiet --non-interactive
rynthv -cli dictionary list --json --quiet --non-interactive
```

- 需使用 GPU 推理时，先执行 `rynthv -cli doctor --probe-webgpu --json` 获取可用的 `id` JSON 对象（常规 `doctor` 不探测 GPU）。
- 不确定某条子命令的精确参数、输入 Schema 或参数范围时，直接自检解析器：
  ```sh
  rynthv -cli schema note.add --json
  rynthv -cli capabilities --json
  ```
- 缺失路径时通过 `config set` 写入共享配置（GUI 与 CLI 共用同一份 `synthesis.json`；传 `none` 可清除某个目录）：
  ```sh
  rynthv -cli config set --sv1-databases /res/sv1 --databases /res/sv2 \
    --flat-databases /res/flat --flat-models /res/flat-models \
    --dictionaries /res/clf-data --compute-backend cpu --json
  ```
- 若声库为 `.sfpk` / `.sfpks`，先 `voice import --input <pkg>`；若为 SV2 / SV2-compatible 且 `voice validate` 报 `MODEL_CONTEXT_REQUIRED`，执行 `voice context <voice_id> --input <context.json>`。

## 3. 调用模式选型

| 场景 | 推荐模式 | 关键约束 |
| :--- | :--- | :--- |
| 单次查询 / 配置 / 单步渲染 | **单次子命令** (`rynthv -cli --json --quiet --non-interactive ...`) | 每次启动重新加载配置与声库 |
| 对同一 `.svp` 的多步原子编辑 | **`project apply`** (`rynthv -cli project apply FILE --input ops.json --json`) | 内存顺序执行，任一步失败全量回滚；`operations` 内**不能**包含 `<project>` 路径与事务级选项 |
| 多步编辑 + 撤销重做 + 多次试听渲染 | **JSON Lines 会话** (`rynthv -cli --stdio`) | **不要**在根命令加 `--json`；编辑留在内存，必须 `project save` 才落盘；复用声学缓存与计算设备 |
| 跨多个工程的独立批处理 | **`batch run`** (`rynthv -cli batch run --input jobs.json --json`) | 每项包含完整参数（含 `<project>`）；非原子提交，不可嵌套 `batch` 或使用 `--input -` |

## 4. 端到端歌声制作与校验闭环

### 步骤 1：准备工程与时间轴
- 新建：`project new song.svp --bpm 128 --time-signature 4/4 --json`
- 旧版导入：若 `project info` 显示 `imported: true`（SVP 153），只可读/渲染，**禁止直接编辑**；先执行 `project clone old.svp editable.svp --json` 升级为 SVP 196。
- 多声部/伴奏：`track add`、`track set`（`--volume` dB、`--pan` `-1..1`）、`audio import`（伴奏偏移用秒）、`tempo set`、`timesig set`。

### 步骤 2：写入音符、分配声库与歌词音素
1. 分配声库：从 `voice list` 取 `database_id`，执行 `voice assign song.svp --track "$TRACK" --voice "$VOICE_ID" --language japanese`（可用 `voice modes "$VOICE_ID"` 确认支持语言、风格名与音素表）。
2. 写音符：用 `midi import`、`note add` 或 `note import`（JSON 数组）。
3. 填歌词（**必须在设置音素之前**）：
   - 词典自动分词填词：`lyrics fill song.svp --track "$TRACK" --language japanese --text "あ い う え"`（给定 `--language` 且未传 `--literal` 时自动启用词典分词并写入音素）。
   - 字面分词：`lyrics fill ... --literal --text "la - br"`（`-` / `ー` 为延音，`br` 为换气）。
   - 若返回 `LYRIC_COUNT`，检查 `error.details` 中的 `notes` 与 `syllables` 数量差。
4. 查与改音素（**必须在 `phoneme adjust` 之前执行 `phoneme set`**）：
   - `note list` 获取持久 `id`（UUID）。
   - `phoneme inspect` 查看 `resolved_phones` 的 `index` 与 `token`。
   - `phoneme set` 覆盖单音符音素；`phoneme adjust --note "$ID" --index 0 --input adj.json` 微调 `leftOffset` / `durationScale` / `strength` 等。

### 步骤 3：调音曲线、音符表现与重录
- 自动曲线：`parameter list` 查看有效参数键、范围和单位；`parameter set` / `parameter curve` / `parameter point` / `parameter transform` 编辑组级曲线（如 `pitchDelta`、`tension`、`breathiness`、`mode:<StyleName>`）。
- 音符级表现：`parameter note` 合并 `expValueX`、`expValueY`、`dF0VbrMod`、`musicalType`（`singing`/`rap`）、`rTone`、`rIntonation`、`accent`、`detune`、`sv1Pitch`。
- 实例级歌声参数：`parameter voice` 调整 `paramLoudness`、`paramGender`、`vocalModePreset`、`vocalModeParams` 等。
- SV2 音高控制与重录：`pitch inspect/export/import` 读写 `pitch_controls` 与 `pitch_delta`；`retake create/select/prune` 管理 `pitch` / `timbre` / `duration` 重录种子。

### 步骤 4：渲染、声学检查与迭代修正
1. 渲染前验证：`project validate song.svp --json` 确认 `resources_ready: true`，以及 `note overlaps song.svp --json` 无同组音符重叠。
2. 片段试渲染与预测分析（无需写 WAV）：
   ```sh
   rynthv -cli render analyze song.svp --from 0 --to 8 --unit beat --compute-backend cpu --json --quiet
   ```
   检查返回中的 `owners[].predicted_pitch`、`owners[].predicted_phonemes`（含精确到秒的音素起止时间）与 `measurements`（`peak_dbfs`、`samples_above_full_scale`、`silence`）。
3. 导出音频并校验实际波形：
   ```sh
   rynthv -cli render song.svp --output out.wav --sample-rate 48000 --channels 2 --bit-depth 32 --compute-backend cpu --json --quiet
   rynthv -cli audio analyze out.wav --f0 --json --quiet
   ```
   单声部时对比 `estimated_f0.frames` 与目标音高曲线；确认无削波后，可用 `--stems --output-dir stems` 导出分轨。

## 5. 必避陷阱与安全规范

1. **两套时间坐标系绝对不可混用**：
   - `note` / `group` / `render` 的 `--at`、`--from`、`--to`、`--duration` 使用**工程绝对时间**（由 `--unit` 控制，默认 `beat`，`1 beat = 705600000 blick`）。
   - `parameter curve` / `parameter point` / `parameter transform` / `pitch` 的时间点一律使用**组内相对 Blick（`local_blick`）**，不受 `--unit` 控制！请从 `note list` 的 `local_onset_blick` 对齐坐标。
   - `audio import` / `audio set` 偏移与时长一律使用**秒**；`tempo` 用四分音符拍数；`timesig` 用 1 基小节号。
2. **覆盖清除链条**：修改 `lyric` 会清空 `phonemes` 覆盖和逐音素属性；修改 `phonemes` 字符串也会清空逐音素 `attributes.phonemes`。顺序必须是：写音符 → 填歌词 → `phoneme set` → `phoneme adjust`。
3. **共享组定义防护 (`SHARED_GROUP`)**：当 `group list` 中 `reference_count > 1` 时，任何修改音符/歌词/音素/曲线/重录的操作默认被拒绝；若只想改当前片段，先执行 `group independent`，**随后必须重新 `note list`** 获取新生成的音符 UUID（或显式传 `--shared` 同步修改所有链接实例）。
4. **`--stdio` 会话状态机**：
   - `project new` 在当前无打开工程时会自动打开新工程；已有打开工程时不会自动切换。
   - 会话中的编辑只存在于内存（`saved: false`），必须显式调用 `project save` 持久化，否则 `exit` / `close` 会报 `UNSAVED_CHANGES`（除非 `project close --discard`）。
   - 需要内联数据传 `--input -` 时，在 JSON Lines 请求对象中提供 `"input": ...` 字段，而非写入 stdin 下一行。
5. **写入与并发保护**：
   - 写入前先用 `--dry-run` 预检；导出/新建/克隆/导入上下文默认禁止覆盖已有文件，仅支持输出的文件写命令才允许传 `--overwrite`（对其他命令传 `--overwrite` 会报 `INPUT_INVALID`）。
   - 多步骤协作时从 `project info` 读取 `revision`，在下次写命令传入 `--expect-revision <SHA256>`；遇到 `PROJECT_CONFLICT`（退出码 `4`）时重新拉取状态并重规划，切勿强行覆盖。
