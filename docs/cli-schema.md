# CLI 机器接口

命令描述是可执行参数解析器生成的。直接查询 `capabilities --json`、`schema --json` 或 `schema note.add --json`。用 `doctor --json` 查看平台与资源状态，用 `doctor --probe-webgpu --json` 获取适配器 ID。

## 版本与结果

每条 `--json` 命令在 stdout 输出一个 UTF-8 JSON 对象。stderr 用于进度及运行日志。全局 `--quiet` 抑制进度及人工输出，JSON 结果仍保留。命令层级使用点号表示，`track list` 返回 `"command":"track.list"`。常规帮助和版本输出遵循命令行文本约定。

```json
{"schema_version":1,"ok":true,"command":"note.list","result":{"notes":[]},"warnings":[]}
```

```json
{"schema_version":1,"ok":false,"command":"note.set","error":{"code":"NOTE_NOT_FOUND","message":"No visible note matches the UUID and filters","details":{"note_id":"missing"}},"warnings":[]}
```

`schema_version` 表示传输格式。调用方应在支持的版本上按需要读取字段，并允许同版本添加字段。音符和轨道 UUID 在项目保存后保持稳定。旧工程缺失的引用或音符 ID 会以当前文件内容为基准确定性补齐。重名轨道需要 UUID。重复组定义可能导致引用歧义，加载时会拒绝该文件。

| 退出码 | 含义 |
| --- | --- |
| 0 | 成功，或显示帮助、版本 |
| 1 | 无法向客户端写出结果等进程 I/O 错误 |
| 2 | 命令用法、输入数据或歌词数量有误 |
| 3 | 工程、资源、渲染、导出或批量任务错误 |
| 4 | 工程并发冲突、未保存状态或输出已存在 |
| 130 | 请求被取消 |

常见诊断包括 `PROJECT_CONFLICT`、`PROJECT_LOAD`、`INPUT_INVALID`、`SHARED_GROUP`、`VOICE_NOT_FOUND`、`VOICE_NOT_ASSIGNED`、`MODEL_CONTEXT_REQUIRED`、`DICTIONARY_REQUIRED`、`LYRIC_COUNT`、`COMPUTE_UNAVAILABLE`、`SYNTHESIS_FAILED`、`NO_AUDIO`。修复操作应使用 `error.code` 和 `error.details`，消息可能随语言及模型改变。

## 写入与并发

读写工程使用 SVP 校验、原子文件替换和同路径锁。非会话编辑基于加载时的 SHA-256 保存。可将 `project info` 的 `result.revision` 作为 `--expect-revision` 传给下一次工程编辑，防止计划基于过时的查询。GUI 普通保存也会检查打开工程时的修订。出现冲突后先读取变化，再决定如何重做操作。

`project apply` 和 `batch apply` 接受一个 `operations` 数组，数组元素是去掉工程路径的项目编辑参数。操作在内存按顺序处理，全部成功才保存一次。失败索引从 0 开始。`batch run` 接受包含完整路径的命令数组并依次执行，每项各自发布。示例见 [batch.json](../examples/cli/batch.json)。

`--dry-run` 验证适用的写入命令而不发布结果。渲染 dry-run 检查输出配置和参数，不启动神经推理。自动化环境不会隐式进入 REPL。`--input -` 表示有意读取重定向的 stdin。新建、克隆和导出默认拒绝覆盖已有文件。可在支持的命令中显式加 `--overwrite`。工程同路径编辑使用上述修订机制。

## JSON Lines

`rynthv -cli --stdio` 每行接受一个带字符串或整数 `id` 的请求。数组 `args` 使用普通命令的参数顺序。使用 `--input -` 时，`input` 可携带 JSON 文档或 UTF-8 文本。逐行请求和直接输入最多 64 MiB。每个响应带同一 `id`，单次失败后连接仍可继续。

通过 `project open FILE` 打开会话工程后可省略该路径，并用 `project select --track UUID --group UUID` 维护编辑目标。会话编辑留在内存，`project undo` 和 `project redo` 可回滚和重做。执行 `project save` 后持久化。发送 `{"id":"stop","args":["exit"]}` 可正常退出。若工程有未保存修改，先保存或执行 `project close --discard`。
