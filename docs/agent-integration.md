# Agent 集成

给 Agent 提供可运行的 `rynthv`、资源目录及一个可写工作目录。可以在同一环境运行 [双声部制作示例](../examples/cli/make_song.py)。它用 CLI 创建旋律、分配声库和日语音节、设置调音曲线、分析一段主唱并导出成品。

```sh
rynthv -cli config set --sv1-databases /resources/sv1 --dictionaries /resources/clf-data
rynthv -cli voice list --json
python3 examples/cli/make_song.py ./demo "$VOICE_ID"
```

脚本还接受 `--compute-backend webgpu`、`--revision` 和 `--backing`。`VOICE_ID` 选择支持日语的声库。若用 SV2，先完成 [资源与上下文配置](headless.md#声库与词典)。

## Shell 批量调用

```sh
rynthv -cli project new draft.svp --json
rynthv -cli project apply draft.svp --input examples/cli/batch.json --dry-run --json
rynthv -cli project apply draft.svp --input examples/cli/batch.json --json
printf '%s\n' '[{"at":2,"duration":1,"pitch":"F4","lyric":"う"}]' |
  rynthv -cli note import draft.svp --input - --json
rynthv -cli note list draft.svp --json
```

`batch run --input examples/cli/song.json` 展示多个独立请求。对同一工程需要原子提交时，使用 `project apply`。Agent 读到 `PROJECT_CONFLICT` 应重新读取轨道、实例和音符，针对当前修订重新规划。

## Python 使用持久会话

```python
import json
import subprocess

with subprocess.Popen(
    ["rynthv", "-cli", "--stdio"],
    text=True, encoding="utf-8", stdin=subprocess.PIPE, stdout=subprocess.PIPE,
) as process:
    def call(request_id, *args, **payload):
        request = {"id": request_id, "args": list(args), **payload}
        process.stdin.write(json.dumps(request) + "\n")
        process.stdin.flush()
        reply = json.loads(process.stdout.readline())
        assert reply["id"] == request_id
        if not reply["ok"]:
            raise RuntimeError(reply["error"])
        return reply["result"]

    call(1, "project", "open", "draft.svp")
    call(2, "note", "import", "--input", "-", input=[
        {"at": 4, "duration": 1, "pitch": "G4", "lyric": "え"},
    ])
    call(3, "project", "save")
    call(4, "exit")
```

分析循环可用 `render analyze` 取得音高和时序预测，用 `audio analyze` 检查实际采样和削波。建议通过音符 UUID 应用修正。对同一声库重复渲染时，保持 stdio 会话可以复用设备和声学状态。
