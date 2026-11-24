# 机器协议、错误恢复与 JSON 数据模式

## 1. JSON 输出信封与退出码

所有 `--json` 命令和 `--stdio` 响应均返回固定格式的单行 UTF-8 JSON 对象（`schema_version: 1`）：

```json
{"schema_version": 1, "ok": true, "command": "note.list", "result": {"notes": []}, "warnings": []}
```

```json
{"schema_version": 1, "ok": false, "command": "note.set", "error": {"code": "NOTE_NOT_FOUND", "message": "...", "details": {"note_id": "missing"}}, "warnings": []}
```

| 退出码 | 常见 `error.code` | Agent 恢复动作 |
| :--- | :--- | :--- |
| `0` | — | 成功（或显示 help/version） |
| `1` | I/O 失败 | 检查 stdout/文件写权限或子进程管道状态 |
| `2` | `USAGE`、`INPUT_INVALID`、`LYRIC_COUNT` | 查询 `schema <cmd> --json` 修正参数；`LYRIC_COUNT` 按 `details.notes` 与 `details.syllables` 重切词 |
| `3` | `PROJECT_IMPORT_REQUIRED` | 对 SVP 153 执行 `project clone` 转为 SVP 196 后再编辑 |
| `3` | `SHARED_GROUP` | 先 `group independent`（并重新 `note list`）或显式加 `--shared` |
| `3` | `VOICE_NOT_FOUND` / `VOICE_NOT_ASSIGNED` | `voice list` 查询 `database_id` 并执行 `voice assign` |
| `3` | `MODEL_CONTEXT_REQUIRED` | 执行 `voice context <voice_id> --input context.json` |
| `3` | `DICTIONARY_REQUIRED` / `DICTIONARY_RESOURCE` | `config set --dictionaries <clf-data>` |
| `3` | `LYRIC_UNRESOLVED` | 改用 `--literal` 分配或通过 `phoneme set` / `dictionary set` 指定读音 |
| `3` | `COMPUTE_UNAVAILABLE` / `SYNTHESIS_FAILED` | 运行 `doctor --probe-webgpu` 或回退 `--compute-backend cpu` |
| `3` | `NO_AUDIO` | 检查选区 `--from`/`--to`、音符静音、轨道 `mute`/`solo` |
| `4` | `PROJECT_CONFLICT` | 文件被并发修改或 `--expect-revision` 过期：重新 `project info` / `note list` 后重规划 |
| `4` | `OUTPUT_EXISTS` | 确认可覆盖后加 `--overwrite` |
| `4` | `UNSAVED_CHANGES` | `--stdio` 中先执行 `project save` 或 `project close --discard` |
| `130` | `CANCELLED` | 收到 Ctrl+C 中断信号，清理未完成任务 |

## 2. `--stdio` JSON Lines 协议

启动：`rynthv -cli --stdio`（单行请求与内联 `input` 上限均为 **64 MiB**）。

- **请求格式**：`{"id": <string|int>, "args": [<string>...], "input": <optional JSON|string>}`
  - `args` 不含 `rynthv -cli`。
  - 当命令行含 `--input -` 时，直接从请求的 `input` 字段读取内容（字符串按 UTF-8 原文，其他 JSON 值按 JSON 序列化）。
- **会话生命周期**：
  - `project open FILE` 打开工程进内存；此后同路径命令可省略 `<project>` 位置参数。
  - `project new FILE` 在当前无打开工程且非 `--dry-run` 时会自动打开新工程。
  - `project select --track UUID [--group REF_UUID]` 设置默认轨道/组；执行 `track add/copy` 或 `group add/copy/link/split/move` 后会话焦点会自动切换到新对象。
  - 所有编辑留在内存（`saved: false`），支持 `project undo` / `project redo`；`render` 与查询直接读取内存状态；必须 `project save` 才落盘。
  - 正常退出发送 `{"id": "stop", "args": ["exit"]}`（若存在未保存修改须先 `project save` 或 `project close --discard`）。

## 3. 事务与批处理 JSON 格式

### `project apply` / `batch apply`（单工程原子事务）
```json
{
  "operations": [
    ["tempo", "set", "--beat", "0", "--bpm", "128"],
    ["note", "add", "--at", "0", "--duration", "1", "--pitch", "C4", "--lyric", "あ"]
  ]
}
```
- 每个元素自动在下标 `2` 插入 `<project>` 路径，**切勿在子操作里重复写工程路径**。
- 子操作禁止使用 `--dry-run`、`--overwrite`、`--expect-revision`、`--input -`（请放在外层 `project apply` 命令上）。
- 任一步失败会全量回滚，并在 `error.details.operation_index` 标明 0 基失败序号。

### `batch run`（多工程独立批处理）
```json
[
  ["project", "new", "draft.svp", "--bpm", "128"],
  ["project", "apply", "draft.svp", "--input", "ops.json"]
]
```
- 必须包含完整参数（含 `<project>` 路径），逐条执行各自发布；禁止调用 `batch` 或 `--input -`。

## 4. 各编辑命令输入 JSON 模式

### `note import`（`deny_unknown_fields`）
```json
[
  {"at": 0, "duration": 1, "pitch": "C4", "lyric": "あ", "language": "japanese", "mute": false},
  {"at": "1", "duration": "1", "pitch": 62, "phonemes": "i"}
]
```

### `phoneme adjust`（`deny_unknown_fields`）
```json
{"leftOffset": -0.03, "position": 0.5, "activity": 0.5, "strength": 0.2, "durationScale": 1.25}
```
有效范围：`position` `[0, 1]`，`activity` `[0, 1]`，`strength` `[-1, 1]`，`durationScale` `[0.2, 1.8]`，`leftOffset` 为相对音符起始的秒数。

### `parameter curve`
```json
[[0, 0.1], [705600000, 0.25], [1411200000, 0.0]]
```
格式：`[[local_blick, value], ...]`，其中 `local_blick` 必须严格递增。

### `pitch import` / `pitch export`
```json
{
  "pitch_delta": {"points": [0, 15.0, 705600000, 0.0]},
  "pitch_controls": [
    {"type": "point", "pos": 0, "pitch": 60.5, "id": "1001"},
    {"type": "curve", "pos": 705600000, "pitch": 62.0, "id": "1002", "points": [0, 0.0, 352800000, 0.5]}
  ],
  "system_pitch_delta": null
}
```
注：导出中的 `Automation`（`pitch_delta`）内部 `points` 为平铺整数/浮点交错数组 `[blick0, val0, blick1, val1, ...]`，而 `parameter curve` 输入为二维点阵 `[[blick, val], ...]`。

### `parameter note`（允许键白名单，传 `null` 清除可选属性）
`expValueX` `[-1, 1]`、`expValueY` `[-1, 1]`、`dF0VbrMod` `[0, 2]`、`rTone` `[-5, 1]`、`rIntonation` `[-0.5, 0.5]`、`tNoteOffset`（秒）、`evenSyllableDuration`（布尔）、`musicalType`（`"singing"` | `"rap"`）、`accent`（`""` | `"1"`..`"5"`）、`sv1Pitch`（对象或 `null`）、`detune`（音分浮点数）。

### `parameter voice`（实例 `GroupRef.extra.voice` 补丁，传 `null` 清除）
`paramLoudness` `[-12, 12]` dB、`paramTension` `[-1, 1]`、`paramBreathiness` `[-1, 1]`、`paramGender` `[-1, 1]`、`paramToneShift` `[-400, 400]` cents、`transposeCents` `[-50, 50]` cents、`transposeSemitones` `[-12, 12]`、`vocalModeInherited`（布尔）、`vocalModePreset`（字符串）、`vocalModeParams`（`{"<StyleName>": {"pitch": 0..150, "timbre": 0..150, "pronunciation": 0..150}}`）。
