# RynthV Headless CLI 命令参考

所有命令均需以 `rynthv -cli`（或 `--cli`）开头。自动化调用建议固定追加 `--json --quiet --non-interactive`（使用 `--stdio` 时除外）。

## 全局选项

| 选项 | 适用范围 | 说明 |
| :--- | :--- | :--- |
| `--json` | 单次子命令 | stdout 输出单行 JSON 信封 |
| `--quiet` | 单次子命令 | 抑制进度与人工成功输出 |
| `--no-color` | 全局 | 禁用终端颜色 |
| `--non-interactive` | 全局 | 禁止隐式进入 REPL 与交互提示 |
| `--dry-run` | 写入/导出/配置/`save`/`apply` | 校验并预演，不实际修改或发布文件 |
| `--overwrite` | 仅文件输出命令 | 允许覆盖已有输出（`render`、`export`、`midi.export`、`phoneme.export`、`pitch.export`、`project.new`、`project.clone`、`voice.context`）；用于其他命令报 `INPUT_INVALID` |
| `--expect-revision <SHA256>` | 工程修改、`apply`、`project.save` | 校验磁盘已保存 SHA-256 修订，不匹配报 `PROJECT_CONFLICT` |
| `--stdio` | 根命令（独占） | 启动 JSON Lines 持久会话；**不能**与 `--json` 同时传给根进程 |

## 时间、音高与语言约定

- **时间单位 (`--unit`)**：`beat`（默认，四分音符 = `705600000` Blick）、`blick`（非负整数）、`second`（经工程 `TimeAxis` 换算）、`bar`（一基 `BAR:BEAT`，拍号分母决定拍长；用作 `--duration` 时必须为正整数小节数）。
- **组内相对坐标 (`local_blick`)**：`parameter curve` / `parameter point` / `parameter transform` 及 `pitch` 系列始终使用组局部 Blick（从该组原点起算），**不受 `--unit` 影响**。请使用 `note list` 返回的 `local_onset_blick` 对齐。
- **音高格式**：MIDI `0–127` 或音名（`C4=60`，支持 `#`、`♯`、`b`、`♭`，如 `D#4`、`Bb4`）。
- **语言 (`--language`)**：`english`、`mandarin`、`japanese`、`spanish`、`cantonese`、`korean`（亦接受音素集 ID 如 `japanese-romaji`、`mandarin-xsampa`、`english-arpabet` 等）。

## 发现与环境命令

- `capabilities`：返回版本、协议版本、命令完整 Schema、已安装声库清单与特性开关。
- `schema [COMMAND]`：返回解析器生成的参数定义、输入 JSON Schema、参数限制与退出码表（例：`schema note.add`）。
- `doctor [--probe-webgpu]`：检查平台、显示环境、配置路径、资源根目录状态与已发现声库数量；传 `--probe-webgpu` 枚举可用计算适配器 `id` 对象。

## 工程、批处理、轨道与组

- `project new PROJECT [--bpm NUM] [--time-signature N/D]`
- `project info PROJECT`：返回 `revision`、`duration_seconds`、`render_config` 与 `tracks` 摘要。
- `project inspect PROJECT`：返回完整规范化 SVP 文档与 `info`。
- `project validate PROJECT`：离线验证工程结构、伴奏文件、同组重叠音符、声库/上下文/词典就绪状态（`resources_ready`）。
- `project clone PROJECT DESTINATION`：生成新 UUID 的独立副本；**SVP 153 必须先克隆为 SVP 196 再编辑**。
- `project apply PROJECT --input FILE` / `batch apply PROJECT --input FILE`：原子事务提交。
- `batch run --input FILE`：顺序执行多个带完整参数的独立命令（不可嵌套 `batch`，不可用 `--input -`）。
- **会话专用命令**（仅 `--stdio` / REPL）：`project open PROJECT`、`project save`、`project status`、`project close [--discard]`、`project undo`、`project redo`、`project select --track TRACK [--group REF]`。
- `track list PROJECT` / `track add PROJECT --name NAME [--instrumental]` / `track delete PROJECT --track TRACK` / `track copy PROJECT --track TRACK [--name NAME]`
- `track set PROJECT --track TRACK [--name] [--volume dB] [--pan -1..1] [--mute true|false] [--solo true|false] [--index 1..N]`
- `group list PROJECT [--track TRACK]`：返回引用 `id`、`group_id`（定义 ID）、`reference_count` 与起止区间。
- `group add PROJECT --name NAME [--track] [--at] [--duration] [--unit]`（默认起点 `0`，默认时长 `32` 拍）。
- `group delete PROJECT --group REF [--track]`（主组不可单独删除）。
- `group copy|link|move PROJECT --group REF [--track] [--to-track] [--at] [--unit]`
- `group independent PROJECT --group REF [--track]`：为共享引用创建独立定义，**执行后须重新 `note list` 获取新音符 UUID**。
- `group split PROJECT --group REF --at POS [--track] [--unit]` / `group merge PROJECT --group REF --with NEXT_REF [--track]`
- `group set PROJECT --group REF [--track] [--name] [--shared] [--semitones -127..127] [--mute true|false]`（仅改 `--name` 会校验共享定义）。

## 音符、歌词与音素

通用目标筛选器 `<target>`：`[--track TRACK] [--group REF] [--note UUID] [--from POS] [--to POS] [--unit UNIT] [--min-pitch P] [--max-pitch P]`。所有写操作均支持 `--shared`。

- `note list PROJECT <target>` / `note overlaps PROJECT <target>`
- `note add PROJECT --at POS --duration DUR --pitch PITCH [--lyric] [--phonemes] [--language] [--mute true|false] [--shared] <target>`
- `note import PROJECT --input FILE [--track] [--group] [--unit] [--shared]`
- `note set PROJECT [--at] [--duration] [--pitch] [--lyric] [--phonemes] [--language] [--mute true|false] [--shared] <target>`（必须命中唯一音符）
- `note move PROJECT --at POS [--shared] <target>`（必须命中唯一音符）
- `note copy PROJECT [--at POS] [--shared] <target>`（给定 `--at` 时将首个音符对齐至 `--at` 并整体平移复制）
- `note delete PROJECT [--shared] <target>` / `note transpose PROJECT --semitones INT [--shared] <target>`
- `note quantize PROJECT --grid N/D [--shared] <target>` / `note split PROJECT --at POS [--shared] <target>`（单音符） / `note merge PROJECT [--shared] <target>`（同实例同音高相邻或重叠）
- `lyrics list PROJECT <target>`
- `lyrics fill PROJECT [--language LANG] [--text STR | --input FILE] [--syllabify | --literal] [--shared] <target>`
  - 传入 `--language` 且未传 `--literal` 时自动启用词典分词并写入音素；`--literal` 按空白字面分配（`-`、`ー` 延音，`br` 换气）。
- `phoneme inspect PROJECT <target>` / `phoneme export PROJECT --output FILE [--shared] <target>`
- `phoneme set PROJECT --value STR [--shared] <target>`（单音符；传 `""` 清除覆盖）
- `phoneme adjust PROJECT --index N --input FILE [--shared] <target>`（单音符，`--index` 从 0 起算）

## 调音曲线、音符表现、声库参数与重录

### 组级连续参数 (`parameter set/curve/point/transform`)

| 参数键 (`--name`) | 别名 | 范围 | 默认值 | 单位 |
| :--- | :--- | :--- | :--- | :--- |
| `pitchDelta` | `pitch` | `[-1200, 1200]` | `0` | cents |
| `vibratoEnv` | `vibrato` | `[0, 2]` | `1` | normalized |
| `loudness` | — | `[-48, 12]` | `0` | dB |
| `tension` | — | `[-1, 1]` | `0` | normalized |
| `breathiness` | — | `[-1, 1]` | `0` | normalized |
| `voicing` | — | `[0, 1]` | `1` | normalized |
| `gender` | — | `[-1, 1]` | `0` | normalized |
| `toneShift` | `tone-shift` | `[-400, 400]` | `0` | cents |
| `mouthOpening` | — | `[-1, 1]` | `0` | normalized |
| `mode:<StyleName>` | — | `[-150, 150]` | `0` | percent |

- `parameter list PROJECT [--track] [--group]`
- `parameter set PROJECT --name KEY --value NUM [--track] [--group] [--shared]`
- `parameter curve PROJECT --name KEY --input FILE [--track] [--group] [--shared]`
- `parameter point PROJECT --name KEY --at LOCAL_BLICK [--value NUM | --delete] [--track] [--group] [--shared]`
- `parameter transform PROJECT --name KEY [--from LOCAL_BLICK] [--to LOCAL_BLICK] [--scale N] [--offset N] [--smooth 0..100] [--reset] [--track] [--group] [--shared]`
- `parameter note PROJECT [--input FILE] [--shared] <target>`（不传 `--input` 为只读查询）
- `parameter voice PROJECT [--input FILE] [--track] [--group]`（编辑实例 `GroupRef.extra.voice`，不传 `--input` 为只读查询）
- `pitch inspect PROJECT [--track] [--group]` / `pitch export PROJECT --output FILE [--track] [--group]` / `pitch import PROJECT --input FILE [--track] [--group] [--shared]`
- `retake list PROJECT <target>` / `retake create PROJECT [--component pitch|timbre|pitch-and-timbre|duration|all] [--seed U32] [--shared] <target>` / `retake select PROJECT --id U32 [--shared] <target>` / `retake prune PROJECT [--shared] <target>`（均为单音符操作）

## 声库、词典、配置与媒体渲染

- `voice list` / `voice scan` / `voice inspect VOICE [--revision N]` / `voice validate VOICE [--revision N]` / `voice modes VOICE [--revision N]`
- `voice import --input FILE`（`.sfpk` / `.sfpks`）/ `voice context VOICE [--revision N] --input FILE`（导入 SV2 上下文）
- `voice assign PROJECT --voice VOICE [--revision N] [--language LANG] [--track] [--group] [--shared]`
- `dictionary list` / `dictionary lookup --language LANG --word WORD` / `dictionary project PROJECT`
- `dictionary set PROJECT --name NAME --language LANG [--word W] [--phones STR] [--assign] [--track] [--group] [--shared]` / `dictionary delete PROJECT --name NAME`
- `config show` / `config set [--databases] [--sv1-databases] [--flat-databases] [--flat-models] [--dictionaries] [--compute-backend auto|cpu|webgpu] [--compute-adapter ID_OR_auto]`（目录传 `none` 可清除）
- `tempo list|set|delete`（`--beat`、`--bpm`）/ `timesig list|set|delete`（`--bar`、`--value N/D`）
- `midi inspect SOURCE` / `midi import PROJECT --input FILE [--tracks 0,1] [--time-map]` / `midi export PROJECT --output FILE [--track]`
- `audio import PROJECT --input FILE [--name] [--track] [--offset SEC]` / `audio set PROJECT --track TRACK [--offset SEC] [--source-offset SEC] [--duration SEC]` / `audio analyze SOURCE [--f0]`
- `render [PROJECT]` / `export PROJECT`：`[--track] [--group] [--from] [--to] [--unit] [--output FILE | --stems --output-dir DIR] [--vocals-only] [--compute-backend auto|cpu|webgpu] [--compute-adapter ID_OR_auto] [--sample-rate 44100|48000|96000] [--channels 1|2] [--bit-depth 16|24|32] [--cache-mib N] [--seed U32]`
- `render analyze PROJECT`：同上作用域、范围与计算参数，**禁止**传 `--output`、`--output-dir`、`--stems`、`--sample-rate`、`--channels`、`--bit-depth`。
- **环境变量**：`RYNTHV_THREADS`（`1..=256`，默认最多 8 个 CPU 推理工作线程；仅在首次合成前读取）。
