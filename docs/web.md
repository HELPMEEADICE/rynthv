# RynthV Web

RynthV Web 由同一套 Rust 编辑器、SVP 编解码器、模型解析器和合成引擎构建。eframe WebRunner 管理 Canvas、HiDPI、IME、剪贴板和输入。模型目录扫描、压缩包解包、合成、伴奏解码和离线导出在 Dedicated Worker 中执行。Web Audio 的 AudioWorklet 消费 PCM，按 AudioContext 设备采样率播放。整站使用静态文件，用户资源保存在浏览器当前源中。

## 浏览器要求

- 使用 HTTPS 或 `http://localhost`。浏览器须开放 OPFS、IndexedDB、AudioWorklet 和 Dedicated Worker。
- WASM 使用 SIMD128 与 JS Promise Integration（JSPI）。Chromium 137 起默认启用 JSPI。其他浏览器请核对 JSPI 支持，启动页会检测能力。浏览器同时需要 WebGPU 或 WebGL 2 绘制编辑器。[JSPI 支持表](https://wasm-bindgen.github.io/wasm-bindgen/reference/jspi.html)
- **CPU** 在单个 Worker 内执行 WASM SIMD128。当前 Web 构建为单线程，不使用 SharedArrayBuffer 或线程版 Rayon。COOP/COEP 对常规运行可选，跨源隔离不会自动开启多线程。
- **Auto** 优先尝试 Worker WebGPU，根据设备限制和算子工作量调度。**WebGPU** 请求计算专用设备。首次 GPU 批次完成后，小算子也按工作量阈值调度，减少频繁提交和回读。初始化和执行错误会显示在渲染面板。设置显示实际 CPU／WebGPU／混合路径及 GPU 提交和回读耗时。编辑器图形后端与推理设备分开。[WebGPU API](https://developer.mozilla.org/en-US/docs/Web/API/WebGPU_API)

## 构建与静态部署

安装仓库锁定的 Rust 1.99.0、Python 3.11 或更新版本和匹配 `Cargo.lock` 的 wasm-bindgen CLI。

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked wasm-bindgen-cli --version 0.2.129 --bin wasm-bindgen
python packaging/web/build.py --check
python packaging/web/build.py
python -m http.server 8080 --bind 127.0.0.1 --directory dist/web
```

`--check` 执行 WASM 编译检查和严格 Clippy。`--debug` 生成开发包。默认使用 release、Thin LTO 和 Rust 的性能优化级别，保留推理内核的速度。脚本固定字体版本与 SHA-256，构建时缓存字体与许可证，运行时字体从同站点加载。

上传 `dist/web/` 全部内容，或解压 `dist/rynthv-web.zip` 到 GitHub Pages、Cloudflare Pages 或 Nginx。入口 `index.html` 引用同一个 `assets/<内容哈希>/` 目录，主页面模块、Worker、AudioWorklet、WASM 和字体都使用该目录内的相对路径，支持站点子目录。发布时先上传本次资源目录，再切换 `index.html`。可配置 HTTP 缓存时，为入口 HTML 设置 `Cache-Control: no-cache`，为内容哈希目录设置 `Cache-Control: public, max-age=31536000, immutable`。从旧的平铺包更新时，清除部署缓存中的旧入口和旧资源 URL。

确保服务器为 `.wasm` 返回 `application/wasm`，为 `.js` 返回 JavaScript MIME 类型。访问发布目录的带尾部斜线地址。使用内容安全策略时允许同源模块脚本和 Worker，并为 WebAssembly 编译配置 `wasm-unsafe-eval`。发布包包含字体及其许可证。

WebAssembly 工作流在 PR、main 提交和手动触发时生成部署 ZIP，在 Release 发布时把相同名称的文件上传到对应 Release。[WebAssembly 工作流](https://github.com/HELPMEEADICE/rynthv/actions/workflows/web.yml)

## 导入、合成和保存

1. 在 **管理歌声** 或 **设置 → 歌声资源** 导入 `.svpk`、`.sfpk` 或 `.sfpks`。也可选择已解包的 SV1／SV2 `databases`、Flat 声库或 Base Model 目录。需要上下文的歌声可导入便携 JSON，或为所选歌声手动输入已有的 `identity`。
2. 导入 `clf-data.zip`、`clf-data.7z`，或选择解包后的词典目录。自定义工程发音仍在词典面板编辑。
3. 使用文件菜单或拖拽导入 `.svp`、MIDI 与伴奏。旧工程的外部伴奏需要重连到所选本地文件。一起拖入带相对路径引用的 SVP 与同目录伴奏可以保留关联。
4. 指定音轨歌声，编辑音符、歌词、音高、音素或参数。Worker 优先准备播放位置，后台补齐其余区间。人声和伴奏在各自需要的区间就绪后参与分块混音。定位会更新当前任务的优先级。编辑会取消过期任务，并复用未变的音符组、已完成的区间和模型。首次使用须点击编辑器或按键以启用音频。渲染面板显示音符组进度、首块 PCM 和各阶段耗时。
5. 在渲染面板选择混音或分轨，在导出完成后点击对应 WAV 下载按钮。**文件 → 保存** 将工程放入浏览器工程列表。**下载 SVP** 生成可以带走的工程文件。伴奏引用使用相对文件名，移动到其他设备时同时保留伴奏原文件。

## 浏览器本地存储和限制

- OPFS 保存声库、词典、模型上下文、伴奏和 WAV 导出。工程索引、SVP 快照、自动恢复及工作区配置保存在 IndexedDB。源由协议、域名和端口决定。换域名或端口后，原存储仍属于旧源。
- 工作区每数秒保存改动并在重新打开时恢复。浏览器清理站点数据会删除本地工程和声库。私密模式与设备配额可能限制持久化。**设置 → 歌声资源** 可估算存储用量和清理 WAV 导出缓存。导入的 SVPK 和 Flat 声库副本可从管理歌声中移除。
- 桌面原版会话文件和进程读取需要 Windows x64。浏览器接受便携模型上下文。模型、词典和上下文由用户提供。
- 编辑器和合成 Worker 从同一个已编译的 WASM 模块分别实例化。预览任务在有界队列中优先于等候中的资源读取，正在执行的导入完成后才会处理下一项请求。GPU 回读跨 JSPI 暂停 WASM 调用栈，CPU 渲染在计算检查点让出 Worker 事件循环。首次读取伴奏仍需解码整个音频文件。大型工程还受 WASM 线性内存、GPU 设备限制和预览 PCM 缓冲容量约束。
- 编译与静态检查不会验证平台音频时序、WebGPU 数值或实际浏览器的触控和 IME。以下步骤用于实机确认。

## 实机验证

1. 在目标站点打开编辑器。调整窗口与页面缩放，检查 Canvas、中文和日文输入、剪贴板、快捷键及触控双指缩放。
2. 导入一个 `.svpk`、Flat 声库和 `clf-data`。如有 SV2 资源，额外导入 SV2 目录与该歌声的便携上下文。刷新页面，检查目录和配置留存。
3. 先选择 CPU，录入短句并播放。再选择 WebGPU，确认实际推理状态、GPU 提交及回读计数。用相同工程分别导出 WAV，比较长度、采样率和听感。
4. 加入伴奏、第二条人声音轨和循环区。检查跳转、暂停、静音、增益、声像与分轨导出。合成期间连续编辑并撤销，确认最终播放的是最新工程。
5. 保存工程、下载 SVP、刷新并重新打开。验证自动恢复、模型上下文和伴奏重连。最后在移动浏览器检查布局、长按、双指操作及软键盘。
