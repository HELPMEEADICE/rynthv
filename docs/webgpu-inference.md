# 原生 WebGPU 推理

在 RynthV 的 **设置 → 渲染 → 模型推理** 中选择 Auto、WebGPU 或 CPU。使用 Auto 或 WebGPU 时，可在 **WebGPU 显卡** 中选择用于歌曲渲染的显卡。默认为 **自动选择（独显优先）**。后端和显卡选择保存在 `synthesis.json`。设置页会显示实际使用的显卡和推理状态，合成完成后也会显示实际执行路径。**设置 → 常规 → 图形渲染** 显示编辑器的图形后端。

Auto 优先打开可用的硬件适配器，并根据工作量和设备限制分配算子。GPU 无法初始化时，Auto 使用 CPU 并显示原因。WebGPU 模式要求硬件设备可用，初始化或执行失败时会报告错误。个别算子仍可能在 CPU 上执行，报告会列出原因。设备在批次执行期间出错时，该批次返回错误。Auto 可以在后续工作中继续使用 CPU。

Windows 支持 Vulkan 和 DX12。Linux 支持 Vulkan 和符合计算要求的 GL 适配器。自动选卡时，同类设备优先使用 Vulkan。手动选项绑定到显卡和所列的图形 API。驱动为多张显卡报告相同标识时，这些适配器仅可由程序自动选择。所选适配器不可用时，Auto 使用 CPU，WebGPU 模式报告错误。可用 wgpu 的 `WGPU_BACKEND=vulkan` 或 `WGPU_BACKEND=dx12` 限定诊断或合成所用的 API。计算设备独立于编辑器图形设备，可以在无窗口的进程中创建。页面切换设备时会取消现有合成任务并清空合成缓存。合成工作线程和有界声码器队列共享计算设备，音频输出回调只消费已发布的 PCM。

## 模型覆盖

| 模型系列 | 时长 | 音高与音色 | 声码器 |
| --- | --- | --- | --- |
| SV1 | 原生 SV1 时长网络 | DDS 音高与 DDS 音色网络 | 共用神经声码器 |
| SV2-compatible | 兼容时长网络 | DDS 音高与 DDS 音色网络 | 共用神经声码器 |
| SV2 | SV2 时长及相对注意力 | SV2 F0、音色及条件采样网络 | 共用神经声码器 |
| SynthV Flat | 带 Flat 条件的原生时长网络 | 带 Flat 条件的 DDS 网络 | 包提供的声码器使用共用执行器 |

各系列继续使用同一套 CPU 和 GPU 权重、各自的前处理与时钟。上表描述接入路径。某个声库实际执行了哪些算子，可以在渲染报告的 `compute.activity` 中查看。

GPU 算子包括 FP32 帧批量矩阵乘与仿射、偏置、带步幅和膨胀的 1D 卷积、转置卷积、ReLU、Leaky ReLU、Tanh、Sigmoid、SiLU、LayerNorm、ChannelAffine、Embedding、门控激活、GRU、双向 GRU、多头注意力和相对注意力。注意力的分数、Softmax 和加权求和在同一批提交内执行。形状合适的顺序层和等长条件卷积网络共享设备中间缓冲区。流式条件网络保留输入的感受野历史，每批只回读新就绪的行。GRU 只并行计算同一帧的隐藏通道，并按顺序推进帧。每个流的最终隐藏状态、卷积历史和转置卷积重叠数据可由原有的方式保存和恢复。

CPU 执行池化、范围与标准化参数转换、时长头的 Softmax 后处理、一般流式分支对齐和残差合并、特征构造、采样调度，以及音频采样率上的激励和滤波。声码器位置条件保持 CPU 上的 FP64 三角函数，并批量调度仿射投影和 GRU。Auto 会把小算子分配给 CPU。设备张量限制、长 GRU 序列、过大的注意力分数矩阵也可能触发算子级回退。含变长分支的条件网络沿用原有的分支队列，可逐个调度卷积。

Pipeline 随计算设备复用。模型权重在首次使用时上传，缓存按共享权重分配关联，使用 384 MiB 的权重预算和 64 MiB 的空闲缓冲区预算。报告中的权重缓存和批次缓冲区字节数是 RynthV 跟踪的逻辑分配。物理显存驻留与多批次并发峰值以系统工具为准。

## Headless CLI 与对比方法

```sh
cargo build --release --locked --bin rynthv-synth --features synth-profile
rynthv-synth list-compute-devices --validate
rynthv-synth render-score --voice VOICE --input SCORE.json --wav cpu.wav \
  --compute-backend cpu --repeat 3 --output cpu.json --profile cpu-profile.json
rynthv-synth render-score --voice VOICE --input SCORE.json --wav gpu.wav \
  --compute-backend webgpu --repeat 3 --output gpu.json --profile gpu-profile.json
```

用同一个本地声库、显式 score、采样种子和版本运行以上命令。Flat 使用相同的模型目录。SV2 系列需要相同的模型身份条件。每个输出报告包含声库加载时间、模型准备时间、GPU 初始化时间、首轮和热轮耗时，以及 Duration、F0、音色、声码器等阶段的墙钟时间。需要比较空闲设备初始化和进程启动开销时，每种后端分别启动多个新进程。关闭 `--profile` 可测量去掉 GPU 查询和 CPU 计时插桩开销的吞吐量。

- **长音频吞吐量**。用相同的较长 score 渲染，计算 `duration_seconds / (render_ms / 1000)`。首轮和热轮分开报告。
- **短音首块延迟**。为两条命令都添加 `--stream-frames 128`，用相同短音和长句分别比较 `first_pcm_ms`、`pcm_chunks` 和总耗时。此模式运行编辑器使用的渐进式 PCM 程序。
- **资源和算子**。查看 `gpu_submissions`、`completed_gpu_submissions`、`gpu_dispatches`、`readback_ms`、双向传输量、Pipeline 与权重缓存命中。启用 `--profile` 时，支持时间戳的适配器还报告按内核聚合的 `gpu_kernel_ms`。使用 Windows Performance Recorder 或 Linux 的 `/usr/bin/time -v` 记录进程 CPU 时间和峰值常驻内存，并用系统 GPU 性能工具记录显存。
- **实际执行**。`actual` 为 `cpu`、`webgpu`、`mixed` 或 `idle`。成功回读模型计算结果后才记为完成 GPU 提交。`list-compute-devices --validate` 检查 WGSL 与设备的 Pipeline 编译。渲染报告提供实际数值执行证据。
- **数值验证**。用 `prepare-voice` 找到可执行节点，再用 `run-module --compute-backend cpu|webgpu` 对相同的帧主序张量比较输出。先检查形状与有限值。FP32 模块可从 `abs_error <= 1e-4 + 1e-4 * abs(cpu)` 开始排查。端到端检查 WAV 编码、采样率、输出长度、`pitch.midi` 和 `phone_frame_counts`。同一设备上的短句可用 0.01 半音的最大音高误差和 0.001 满量程的 PCM RMS 误差作为初始排查阈值。波形相位可能放大很小的 F0 差异，比较长波形时要同时检查音高和时长。

## 实测基线

Windows x64，Intel Core i9-14900HX，NVIDIA GeForce RTX 5080 Laptop GPU，驱动 621.14。GPU 列使用 Vulkan。时长为同一进程中第二轮及之后的墙钟时间，单位为毫秒。SV1 使用 KASANE TETO AI 104，Flat 使用 Hatsune Miku AI 200。长句由示例 score 的 5 条记录重复 12 次得到。短音使用 150 ms 的 Mandarin `l a`，输出含模型上下文。

| 任务 | CPU | WebGPU | Auto |
| --- | ---: | ---: | ---: |
| SV1 2.75 秒完整渲染 | 524–552 | 250–253 | 239–245 |
| SV1 27.5 秒完整渲染 | 3,939 | 2,917 | 2,881 |
| SV1 2.75 秒渐进渲染 | 487 | 218 | 207 |
| SV1 短音首块延迟，128 帧预算 | 178–183 | 72 | 68 |
| Flat 2.75 秒完整渲染 | 548 | 232 | 未测 |

- **首轮**。SV1 2.75 秒样本的声库加载约 41–43 ms，模型准备约 70 ms。CPU 首轮 583 ms。Vulkan 初始化 326 ms，首轮 345 ms。DX12 初始化 269 ms，首轮 575 ms，热轮 215 ms。
- **GPU 诊断**。同一 SV1 案例的 Vulkan 性能记录显示首轮上传 75.36 MiB 权重，主机准备耗时 119.30 ms。热轮计数为 1,179 次权重缓存命中、1,060 次 Pipeline 缓存命中、88 次回读和 23.89 MiB 回读数据。热轮回读等待及复制合计 118.08 ms，GPU 时间戳测得卷积合计 35.70 ms。
- **资源**。运行三轮 SV1 2.75 秒完整渲染的进程累计 CPU 时间为 CPU 5.47 秒、Vulkan 1.05 秒，峰值工作集分别为 390 和 334 MiB。按整段进程墙钟对 32 个逻辑线程归一化，平均 CPU 利用率分别为 9.39% 和 2.41%。另对 Vulkan 连续三轮 27.5 秒渲染采样，进程专用显存观测峰值为 342 MiB，共享 GPU 内存观测峰值为 60 MiB。
- **输出**。实测 CPU、Vulkan 与 DX12 WAV 为 48 kHz 单声道 FP32。音频样本数和音素帧数一致，SV1 2.75 秒渐进渲染均发布 4 块。音高最大绝对误差为 0.00000763 半音，PCM RMS 误差最大为 0.0000643。GRU、双向 GRU 和 Embedding 的模块输出形状一致，最大张量绝对误差为 0.000000060。

Vulkan 和 DX12 共用 14 个通过设备验证的 WGSL 入口。端到端实测覆盖 SV1 与 SynthV Flat。SV2、SV2-compatible 和 Linux／ARM64 的实机覆盖有待具备对应环境与声库时验证。
