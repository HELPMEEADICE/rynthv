# Headless 构建与运行

RynthV 的默认构建包含 GUI 和 CLI。无桌面服务器可以构建只含 CLI 的二进制。两种构建使用相同的 SVP 数据模型和原生合成程序。

```sh
sudo apt-get install --no-install-recommends build-essential
cargo build --release --locked --no-default-features --features cli --bin rynthv
install -Dm755 target/release/rynthv "$HOME/.local/bin/rynthv"
env -u DISPLAY -u WAYLAND_DISPLAY rynthv -cli --help
env -u DISPLAY -u WAYLAND_DISPLAY rynthv -cli doctor --json
```

Linux x86-64 和 ARM64 使用仓库指定的 Rust 工具链。纯 CLI 构建的 CPU 路径可在没有 X11、Wayland、桌面门户、系统字体和音频设备的环境中运行。GitHub Actions 的 Headless container 作业在 Ubuntu 24.04 容器里检查依赖并运行 CLI 测试。

默认构建中，`rynthv` 和 `rynthv song.svp` 启动 GUI。`-cli` 与 `--cli` 先于窗口和图形设备初始化处理。在 Windows Release 双模式构建中，入口使用控制台子系统，CLI 继承终端标准流并向 Shell 返回退出码。GUI 以分离进程启动。通过 Explorer 双击时，Windows 可能短暂显示控制台窗口。`--no-default-features --features gui` 可构建采用原 Windows GUI 子系统的专用 GUI。

## 声库与词典

目录可以事先挂载到容器。CLI 与 GUI 读取同一份 `synthesis.json`。Linux 将其保存在 `$XDG_CONFIG_HOME/RynthV`，默认根目录为 `$HOME/.config`。Windows 使用 `%LOCALAPPDATA%/RynthV`。

```sh
export XDG_CONFIG_HOME="$PWD/config"
rynthv -cli config set --sv1-databases /resources/sv1 --dictionaries /resources/clf-data
rynthv -cli config set --databases /resources/sv2
rynthv -cli config set --flat-databases /resources/flat --flat-models /resources/flat-models
rynthv -cli voice list --json
rynthv -cli dictionary lookup --language japanese --word '世界' --json
```

`--sv1-databases` 也接受单个 SV1 NOFS 文件。使用 `voice import --input` 导入 `.sfpk` 或 `.sfpks` 文件。共享 Flat 模型放在 `--flat-models` 指定的目录。版本和缺失依赖可用 `voice list` 查询。`config set` 的任一目录参数可用值 `none` 清除。

```sh
rynthv -cli voice import --input /resources/bank.sfpk
rynthv -cli voice inspect "$VOICE_ID" --json
rynthv -cli voice modes "$VOICE_ID" --json
rynthv -cli voice validate "$VOICE_ID" --json
```

SV2 和 SV2-compatible 声库需要对应声库和模型代次的合法上下文。可以导入已有的 RynthV 上下文文件。Windows 读取漫游配置目录中的上下文，Linux 读取 XDG 配置目录中的上下文。CLI 结果会返回导入状态和代次。

```sh
rynthv -cli voice context "$VOICE_ID" --input /resources/context.json
```

## CPU 和 WebGPU

```sh
RYNTHV_THREADS=4 rynthv -cli render song.svp --compute-backend cpu --output result.wav
rynthv -cli doctor --probe-webgpu --json
rynthv -cli render song.svp --compute-backend webgpu --output gpu.wav
```

CPU 选择跳过 GPU 初始化。Linux 纯 CLI WebGPU 使用 Vulkan loader 与驱动的 ICD，容器需要映射计算设备或对应的 GPU runtime。Windows 使用 Vulkan 或 DX12。选定设备时，将 `doctor --probe-webgpu` 返回的一个 `id` JSON 对象传给 `--compute-adapter`。也可以用 `config set --compute-adapter` 保存选择。普通 `doctor` 不探测 GPU。

`auto` 可在 GPU 不可用时使用 CPU，并在计算诊断中保留原因。强制 `webgpu` 的设备初始化失败会返回错误。算子有 CPU 路径时，实际执行可能为 `mixed`。只混伴奏或命中完整缓存时，当前请求的推理路径可能为 `idle`。

`RYNTHV_THREADS` 在进程第一次需要并行合成时读取，接受 1–256。默认使用最多 8 个推理工作线程。`--cache-mib` 设置长期会话在请求之间保留的声学缓存软上限。模型、词典和计算设备也会在同一 REPL 或 stdio 会话中复用。使用 `voice scan` 重新扫描并释放已缓存的声库和曲线结果。

Ctrl+C 请求取消合成或解码。单个输出采用原子发布。分轨导出逐个发布文件，取消或错误前已发布的分轨保留。
