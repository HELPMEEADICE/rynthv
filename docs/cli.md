# Headless 命令参考

执行 `rynthv -cli --help` 浏览入口。每条命令均提供自己的 `--help`。`rynthv -cli schema note.add --json` 返回同一个解析器生成的参数描述。

| 命令族 | 子命令 |
| --- | --- |
| project | `new`、`info`、`inspect`、`validate`、`clone`、`apply`、`open`、`save`、`status`、`close`、`select`、`undo`、`redo` |
| track | `list`、`add`、`copy`、`delete`、`set` |
| group | `list`、`add`、`delete`、`copy`、`link`、`independent`、`move`、`split`、`merge`、`set` |
| note | `list`、`add`、`set`、`import`、`delete`、`copy`、`move`、`transpose`、`quantize`、`split`、`merge`、`overlaps` |
| lyrics、phoneme | `lyrics list/fill`，`phoneme inspect/export/set/adjust` |
| pitch、parameter、retake | `pitch inspect/import/export`，`parameter list/set/curve/point/transform/note/voice`，`retake list/create/select/prune` |
| tempo、timesig | `list`、`set`、`delete` |
| voice、dictionary | `voice list/scan/inspect/validate/modes/import/context/assign`，`dictionary list/lookup/project/set/delete` |
| midi、audio | `midi inspect/import/export`，`audio import/set/analyze` |
| render、export | `render PROJECT`、`render analyze PROJECT`、`export PROJECT` |
| batch、config | `batch apply/run`，`config show/set` |
| 发现与诊断 | `capabilities`、`schema`、`doctor` |

工程可用 `project inspect` 获取完整 SVP 内容，用 `project validate` 检查资源。已有 SVP 153 可读取、检查、渲染，编辑前用 `project clone original.svp editable.svp` 保存为 196。项目文件中的可往返扩展字段保留。若扩展需要尚未实现的合成器语义，引擎会给出错误。

轨道 UUID 使用主引用 UUID。组选择器使用引用 UUID，`group list` 同时返回定义 UUID。克隆轨道会复制主定义，非主组保持链接。组拆分使用同一组定义的两个可见区间。合并要求同轨、相邻、同定义且设置一致。编辑共享定义前用 `group independent` 建立新定义，或者显式传入 `--shared`。独立化后用 `note list` 重新获取音符 UUID。

## 从 MIDI 制作歌曲

```sh
rynthv -cli midi inspect melody.mid --json
rynthv -cli project new song.svp
rynthv -cli midi import song.svp --input melody.mid --time-map
rynthv -cli track list song.svp --json
rynthv -cli voice assign song.svp --track "$TRACK_ID" --voice "$VOICE_ID"
rynthv -cli note list song.svp --track "$TRACK_ID" --json
rynthv -cli render song.svp --track "$TRACK_ID" --compute-backend cpu --output lead.wav
```

`--tracks` 使用 `midi inspect` 的结果数组中的零基索引。导入器按通道拆分，重叠音符也可能成为独立声部。省略 `--time-map` 会保留原工程的曲速和拍号。MIDI 导出保留可见音符、歌词、曲速和拍号，时间按 960 PPQN 量化。声库、音素、参数曲线、演唱表现与伴奏留在 SVP。

## 从零编排

`VOICE_ID` 使用 `voice list --json` 中已安装、支持日语的声库 ID。以下流程使用默认的人声音轨。也可以执行 [可运行的双声部示例](../examples/cli/make_song.py)。

```sh
rynthv -cli project new song.svp --bpm 128 --time-signature 4/4
rynthv -cli note add song.svp --at 0 --duration 1 --pitch C4
rynthv -cli note add song.svp --at 1 --duration 1 --pitch D4
rynthv -cli voice assign song.svp --voice "$VOICE_ID" --language japanese
rynthv -cli lyrics fill song.svp --language japanese --text 'あ い'
rynthv -cli note list song.svp --json
rynthv -cli phoneme inspect song.svp --note "$NOTE_ID" --json
rynthv -cli phoneme set song.svp --note "$NOTE_ID" --value 'a'
rynthv -cli parameter set song.svp --name breathiness --value 0.25
rynthv -cli parameter point song.svp --name pitchDelta --at 352800000 --value 15
rynthv -cli render song.svp --from 0 --to 2 --unit beat --compute-backend cpu \
  --sample-rate 48000 --bit-depth 32 --output preview.wav --json
rynthv -cli render analyze song.svp --compute-backend cpu --json
rynthv -cli audio analyze preview.wav --f0 --json
rynthv -cli render song.svp --stems --output-dir stems --compute-backend cpu
```

显式给定 `--language` 时，`lyrics fill` 从引擎词典求音节。日语使用已有读音和分词数据。使用 `--literal` 可以手工以空白分隔歌词，并在渲染阶段由引擎解析发音。混合语言可用 `note set --note UUID --language LANGUAGE` 或在 [音符 JSON](../examples/cli/notes.json) 中逐音符指定。填充分配数目不匹配时，`LYRIC_COUNT` 返回音符和音节数。可用 `-` 或 `ー` 延续前一音符，可用 `br` 标记换气。

`phoneme adjust` 读取一个 per-phone JSON 对象，`--index` 从 0 起算，其中 `leftOffset` 是相对音符起始的秒，`durationScale` 是时长比例。`parameter note` 支持合并 SVP 音符属性，如 `dF0VbrMod`、`expValueX`、`expValueY`、`musicalType` 和 `sv1Pitch`。`parameter voice` 编辑实例的 VoiceSettings 和 Vocal Mode。用 `voice modes` 查询实际可用风格和音素表。音高控制点、时长重录仅适用原生 SV2，曲线及音高与音色重录的可用性依声库模型而定。

## 时间、音高与音频

- `--unit beat` 以四分音符为一拍，每拍 705600000 Blick。 `--unit blick` 使用整数。
- `--unit second` 通过工程 TimeAxis 转换，跨变速的音符时长以起止位置分别换算。
- `--unit bar` 的位置为一基的 `BAR:BEAT`，拍号分母决定拍单位。用作时长时，整数表示从当前位置所在小节开始的若干小节长度之和。
- 音符可用 MIDI 0–127 或音名（C4=60）。`detune` 和 `pitchDelta` 用音分，`pitch_controls` 的音高偏差以半音表示。
- 参数曲线的点格式为 `[[local_blick,value],...]`。自动化值单位和编辑范围见 `parameter list`。`pitch export` 和 `pitch import` 使用相同结构。
- `audio import` 引用原音频路径，项目内文件可相对工程存储。伴奏偏移用秒。`track set` 接受 dB 音量和 −1 到 1 的声像。独奏、静音及组可见边界参与混音。
- WAV 支持 44.1、48、96 kHz，单声道或立体声，PCM16、PCM24、Float32。未指定时读取 SVP 导出配置。 `--vocals-only` 排除伴奏。选择片段时，文件的首帧对应范围起点。
- `pitch inspect` 查询工程预期音高及控制数据。`render analyze` 返回引擎预测音高、音素时间和混音采样统计。`audio analyze --f0` 从解码 PCM 估计第一声道的主基频，适用于单声部，并以 Hz 表示。混合声部和伴奏可能产生八度偏差或无结果。
