# RynthV

<p align="center">
  <img src="assets/branding/app-icon.png" alt="RynthV 图标" width="96" height="96">
</p>

使用 Rust + egui 开发的歌声合成引擎。

## 构建与运行

需要 Rust 1.99.0，版本由 `rust-toolchain.toml` 指定。

Windows 和 Linux 均支持原生 x86-64 与 ARM64 构建。GitHub Actions 的 Build 工作流为 Windows 提供 MSI 和 ZIP，为 Linux 提供 AppImage，每种架构独立打包。

### Windows

需要支持 DX12 的显卡。MSVC 目标为 `x86_64-pc-windows-msvc` 和 `aarch64-pc-windows-msvc`，需要为目标架构安装 MSVC 工具和含 `rc.exe` 的 Windows SDK。Windows x86-64 也可以使用 MSYS2 GNU 工具链，其 `windres` 需在 `PATH` 中。

GitHub Actions 生成的 MSI 和 ZIP 静态链接 Visual C++ 运行库。

```powershell
cargo build --release --locked --bins
```

### Linux

Linux 目标为 `x86_64-unknown-linux-gnu` 和 `aarch64-unknown-linux-gnu`，支持 X11 / Wayland，使用 Vulkan 或 OpenGL。Ubuntu 24.04 构建示例：

```sh
sudo apt install build-essential pkg-config libasound2-dev libwayland-dev libxkbcommon-dev \
  fontconfig fonts-noto-cjk libvulkan1 libegl1 libgl1 xdg-desktop-portal xdg-desktop-portal-gtk
cargo build --release --locked --bins
./target/release/rynthv
```

AppImage 在 Ubuntu 24.04 上构建。运行依赖系统的 glibc 和 `libgcc_s.so.1`。编辑器还需要图形驱动、`libasound.so.2`（Ubuntu 上为 `libasound2t64`）、ALSA 音频设备和桌面对应的 XDG 门户后端（KDE 使用 `xdg-desktop-portal-kde`）。可用 `--appimage-extract` 解包，CLI 位于解包目录中的 `usr/bin/rynthv-synth`。

## 声库与配置

首次使用请在设置中手动选择声库目录和发音词典目录，程序不会自动探测资源路径，也不附带声库、词典或用户模型上下文。

| 声库类型 | Windows | Linux |
| --- | --- | --- |
| 一代声库 | 支持 | 支持 |
| 二代声库 | 需要导入模型上下文 | 需要导入模型上下文 |

Windows ARM64 和 Linux 可通过上下文文件或手动输入导入二代声库所需的模型上下文。直接从原编辑器的本机会话文件或在线进程导入需要 Windows x64 的 CPUID 身份格式。

配置目录：Windows 为 `%LOCALAPPDATA%/RynthV`，Linux 为 `$XDG_CONFIG_HOME/RynthV`（未设置时为 `$HOME/.config/RynthV`）。模型上下文在 Windows 保存在 `%APPDATA%/RynthV/context.json`，Linux 保存在上述配置目录的 `context.json`。

命令行合成与诊断工具的用法可通过 `cargo run --release --locked --bin rynthv-synth -- --help` 查看。

CPU / WebGPU 推理设置和性能对比方法见 [原生 WebGPU 推理指南](docs/webgpu-inference.md)。

SynthV Flat 明文声库的导入、模型目录配置、原生合成和兼容范围见 [SynthV Flat 指南](docs/synthv-flat.md)。
