# Android 版 RynthV

Android 版使用 NativeActivity、Rust 和 egui。工程格式、编辑器、MIDI、渐进式歌声合成与 WAV 渲染均与桌面共用。合成在设备上运行。

## 下载和安装

需要 Android 8.0（API 26）或更新版本，并具备 Vulkan 或 OpenGL ES 图形驱动。GPU 模型推理目前使用 Vulkan。

| 设备架构 | APK |
| --- | --- |
| ARM64 手机及平板 | `rynthv-android-arm64-v8a.apk` |
| x86-64 设备 | `rynthv-android-x86_64.apk` |

在 [Android 工作流](https://github.com/HELPMEEADICE/rynthv/actions/workflows/android.yml) 中打开成功运行，从 Artifacts 下载对应架构并解压。正式发布时也可查看 [GitHub Releases](https://github.com/HELPMEEADICE/rynthv/releases) 的 Assets。用 Android 文件管理器打开 APK，按系统提示授予该来源安装权限。

测试包在启动器显示为 **RynthV (Test)**，包名为 `org.rynthv.app.test`。每次 CI 测试构建生成临时签名，更换测试包前请导出需要保留的工程和音频。正式 APK 使用 `org.rynthv.app` 与持久化签名。

## 工程、声库和渲染

1. 从 **文件 → 打开** 选择 SVP，从 **文件 → 导入** 导入 MIDI，或用 **作为音轨导入** 保留当前工程。**导入音频** 支持 WAV、MP3、FLAC 等桌面版已支持的格式。
2. **保存** 使用应用私有目录内的工作副本。以后从 **本机工程** 打开该副本。**另存为** 用系统文件选择器导出新的 SVP 副本。**分享工程** 打开系统分享菜单。
3. 打开歌声管理或设置，选择 SV1 或 SV2 的声库目录、Flat 声库目录、Base Model 目录和词典目录。可以直接导入 `.sfpk` 与 `.sfpks`。需要模型上下文时，选择 **上下文文件** 导入便携 JSON，或使用 **手动输入**。原编辑器进程与本机会话文件的导入功能适用于 Windows x64。
4. 在 **渲染** 选择混音或独立音轨、位深与采样率。合成完成后点击 **导出为文件**，再对生成的 WAV 选择 **保存 WAV** 或 **分享 WAV**。

Android 通过 Storage Access Framework 打开 `content://` 文档。应用通过文件描述符流式读取，先写临时文件再发布到私有目录。目录授权可跨重启保留。同一目录重新导入时会根据大小和修改时间复用已有文件。提供方缺少元数据时可能需要重新读取文件。系统空间不足时，导入会报告错误。

私有工程、导入音频和声库会在应用数据目录中保留，应用卸载或清除数据会移除这些文件。渲染临时副本与分享文件位于 Android 缓存目录，可能由系统清理。SVP 中引用的伴奏仍是独立素材，跨设备传输时请带上音频并在目标设备重新链接。

## 编辑和试听

- 轻触音符进行选择。拖动音符中央移动时间或音高，拖动两端改变时长。用音符绘制工具轻触空白处添加音符。
- 拖动空白区域框选。长按音符打开歌词、重录、剪切、复制、粘贴和删除菜单，也可增减当前选择中的音符。
- 双指捏合调整时间或音高缩放，双指同向移动平移卷帘。开始双指操作时会取消尚未提交的单指拖动。
- 用卷帘工具栏切换音高控制点、自由绘制和网格。用卷帘下方的标签展开音素和参数编辑区域。手机底部的卷帘、音符、歌声、词典、混音及渲染页可随时切换。
- 在手机底部控制播放、暂停、停止、循环、撤销和重做。展开编排可添加轨道，进入混音页可调节各轨音量、声像、静音和独奏。
- 点击歌词或文本编辑区会打开系统输入法。外接键盘和鼠标仍可用于桌面式操作。

应用切到后台或失去 Audio Focus 时暂停试听并保留播放位置。回来后由用户继续播放。蓝牙或 USB 输出设备变化时会重新打开 AAudio 流。生成 WAV 时请保持应用在前台。最新提交的工程编辑会写入恢复快照，系统回收进程后可继续编辑。

## CPU 和 WebGPU

在 **设置 → 音频与 MIDI → 模型推理** 选择 **Auto**、**CPU** 或 **GPU（WebGPU）**。Auto 根据当前适配器和算子调度选择执行路径。显式 WebGPU 在 GPU 初始化失败时报告错误。设置页显示实际执行后端、驱动和缓冲区限制，适合在同一工程上对照 CPU 与 WebGPU。

Android 图形与模型推理使用独立设备。推理的权重缓存、临时缓冲区及注意力分块会根据设备内存和 Vulkan 限制调整。不同 Adreno、Mali、Immortalis、PowerVR 驱动的 WGSL 计算和大型声库峰值内存仍待实机覆盖。若遇到失败，可在记录设备与驱动信息后选择 CPU 继续工作。

从系统选择器导入私有目录的 DNNI 模型使用只读内存映射。重新导入时替换文件路径，已加载模型可安全读完先前的文件版本。

## GitHub Actions

Android 工作流在 GitHub Hosted Runner 上使用 Rust 1.99.0、JDK 17、Gradle 9.6.0、AGP 9.5.2、API 36、Build Tools 36.0.0 和 NDK 28.2.13676358。每个 ABI 独立运行依赖隔离检查、Cargo check、Clippy、链接、Gradle 打包和签名验证。产物检查涵盖应用 ID、入口 Activity、目标 SDK、ABI、ELF LOAD 对齐以及 APK 内未压缩原生库的 16 KiB ZIP 对齐。Gradle 仅包装 GitHub Actions 上预先链接的 Rust 共享库。

推送至 `main` 或更新 PR 会触发 Android 工作流，也可在 Actions 手动运行 **Android**。发布 GitHub Release 会通过 **Build** 工作流运行同一 Android 构建，并在八个预期资产齐全后为该 Release 附加 APK、MSI、ZIP 和 AppImage。

要为 Release 使用正式签名，在仓库的 **Settings → Secrets and variables → Actions** 配置以下 Secrets。请保留同一密钥用于后续覆盖安装。

| Secret | 内容 |
| --- | --- |
| `ANDROID_KEYSTORE_BASE64` | PKCS#12 签名密钥库的 Base64 |
| `ANDROID_KEYSTORE_PASSWORD` | 密钥库密码 |
| `ANDROID_KEY_ALIAS` | 应用签名密钥别名 |
| `ANDROID_KEY_PASSWORD` | 签名密钥密码 |

## 真机验证范围

APK 静态检查可以验证打包和签名。以下场景需要用户在设备上确认。

- ARM64 Android 8 设备与现代 16 KiB 页设备的安装和冷启动。x86-64 设备的启动与渲染。
- 横竖屏、刘海、系统导航、后台与返回前台、进程回收后的工程恢复。检查中文、日文与英文输入法和外接输入设备。
- 新建和再次打开私有工程，选择本机或云端 SAF 文档，导入大型声库，重新链接伴奏，保存和分享 SVP 与 WAV。
- SV1、SV2-compatible、SV2、SynthV Flat 分别使用 CPU 合成并导出 WAV。在有 Vulkan 计算能力的设备上用同一工程对照 WebGPU 实际执行统计和音频结果。
- 多轨试听、蓝牙与 USB 路由、通话中断、恢复后的播放位置，以及触控绘制、长按和双指手势。
