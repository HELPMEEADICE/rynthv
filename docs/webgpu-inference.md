# 原生 WebGPU 推理

在 RynthV 的 **设置 → 渲染 → 模型推理** 中选择 Auto、WebGPU 或 CPU。该选择保存在 `synthesis.json`。设置页会显示推理设备状态，合成完成后也会显示实际执行路径。**设置 → 常规 → 图形渲染** 显示编辑器的图形后端。

Auto 优先打开可用的硬件适配器，并根据工作量和设备限制分配算子。GPU 无法初始化时，Auto 使用 CPU 并显示原因。WebGPU 模式要求硬件设备可用，初始化或执行失败时会报告错误。个别算子仍可能在 CPU 上执行，报告会列出原因。设备在批次执行期间出错时，该批次返回错误。Auto 可以在后续工作中继续使用 CPU。

Windows 使用 DX12。Linux 通过 Vulkan 和符合计算要求的 GL 适配器提供推理。计算设备独立于编辑器图形设备，可以在无窗口的进程中创建。页面切换设备时会取消现有合成任务并清空合成缓存。合成工作线程和有界声码器队列共享计算设备，音频输出回调只消费已发布的 PCM。

## 模型覆盖

| 模型系列 | 时长 | 音高与音色 | 声码器 |
| --- | --- | --- | --- |
| SV1 | 原生 SV1 时长网络 | DDS 音高与 DDS 音色网络 | 共用神经声码器 |
| SV2-compatible | 兼容时长网络 | DDS 音高与 DDS 音色网络 | 共用神经声码器 |
| SV2 | SV2 时长及相对注意力 | SV2 F0、音色及条件采样网络 | 共用神经声码器 |
| SynthV Flat | 带 Flat 条件的原生时长网络 | 带 Flat 条件的 DDS 网络 | 包提供的声码器使用共用执行器 |

各系列继续使用同一套 CPU 和 GPU 权重、各自的前处理与时钟。上表描述接入路径。某个声库实际执行了哪些算子，可以在渲染报告的 `compute.activity` 中查看。

GPU 算子包括 FP32 帧批量矩阵乘与仿射、偏置、带步幅和膨胀的 1D 卷积、转置卷积、ReLU、Leaky ReLU、Tanh、Sigmoid、SiLU、LayerNorm、ChannelAffine、Embedding、门控激活、GRU、双向 GRU、多头注意力和相对注意力。注意力的分数、Softmax 和加权求和在同一批提交内执行。形状合适的顺序层与完整条件网络共享设备中间缓冲区，每批只回读最终结果。GRU 只并行计算同一帧的隐藏通道，并按顺序推进帧。每个流的最终隐藏状态、卷积历史和转置卷积重叠数据可由原有的方式保存和恢复。

CPU 执行池化、范围与标准化参数转换、流式分支对齐和部分残差合并、特征构造、采样调度，以及音频采样率上的激励和滤波。Auto 会把小算子分配给 CPU。设备张量限制、长 GRU 序列、过大的注意力分数矩阵也可能触发算子级回退。增量与流式执行仍在现有边界回读，完整序列的条件网络更容易受益于设备驻留。

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
