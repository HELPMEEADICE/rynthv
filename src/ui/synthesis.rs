use crate::{app::App, tr, trf};
use eframe::egui::*;

pub fn render_status(app: &mut App, ui: &mut Ui) {
    ui.label(RichText::new(tr!("歌声合成")).strong());
    ui.separator();
    ui.label(app.synthesis.status());
    if let Some(progress) = &app.synthesis.progress {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(progress);
        });
    }
    let stats = app.synthesis.stats;
    ui.label(trf!(
        "已完成 {} 组 · 处理中 {} 组 · 等待 {} 组 · 失败 {} 组",
        stats.ready,
        stats.running,
        stats.queued,
        stats.failed
    ));
    if let Some(error) = &app.synthesis.error {
        ui.colored_label(Color32::from_rgb(218, 142, 126), error);
    }
    for (selection, name) in app.synthesis.context_import_targets() {
        if ui.button(trf!("为 {} 导入模型上下文…", name)).clicked() {
            app.synthesis.open_context_import(selection);
        }
    }
    if !app.synthesis.failures.is_empty() {
        ui.collapsing(tr!("失败详情"), |ui| {
            for (owner, error) in &app.synthesis.failures {
                ui.label(trf!("音符组 {}：{}", owner.0, error));
            }
        });
    }
    if (stats.failed > 0 || app.synthesis.error.is_some()) && ui.button(tr!("重试合成")).clicked()
    {
        app.synthesis.retry();
    }
    if let Some(error) = &app.synthesis.recording_error {
        ui.colored_label(Color32::from_rgb(218, 142, 126), error);
    }
    for error in app.synthesis.recording_failures() {
        ui.colored_label(Color32::from_rgb(218, 142, 126), error);
    }
    if let Some(error) = app.synthesis.playback_error() {
        ui.colored_label(
            Color32::from_rgb(218, 142, 126),
            trf!("播放设备：{}", error),
        );
    }
    ui.add_space(10.0);
    if let Some(duration) = app.synthesis.audio_duration() {
        ui.label(trf!(
            "当前混音 {} 秒 · 48 kHz · 立体声",
            format!("{duration:.2}")
        ));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let (first, metrics) = app.synthesis.preview_timing();
        if first.is_some() || metrics.is_some() {
            ui.collapsing(tr!("预览耗时"), |ui| {
                if let Some(first) = first {
                    ui.label(trf!("请求至首个就绪 PCM 块 {} ms", format!("{first:.0}")));
                }
                if let Some(metrics) = metrics {
                    if let Some(first) = metrics.first_block_ms {
                        ui.label(trf!("Worker 至首块 {} ms", format!("{first:.0}")));
                    }
                    ui.label(trf!(
                        "Worker 完成 {} ms · 区间混音 {} ms",
                        format!("{:.0}", metrics.worker_ms),
                        format!("{:.0}", metrics.mix_ms)
                    ));
                    ui.label(trf!(
                        "声库读取 {} ms · 模型准备 {} ms · 伴奏读取 {} ms",
                        format!("{:.0}", metrics.voice_load_ms),
                        format!("{:.0}", metrics.model_prepare_ms),
                        format!("{:.0}", metrics.recording_ms)
                    ));
                    ui.label(trf!(
                        "时长 {} ms · 音高 {} ms · 音色 {} ms · 声码器 {} ms",
                        format!("{:.0}", metrics.stages.duration_ms),
                        format!("{:.0}", metrics.stages.pitch_ms),
                        format!("{:.0}", metrics.stages.timbre_ms),
                        format!("{:.0}", metrics.stages.vocoder_ms)
                    ));
                }
            });
        }
    }
    ui.collapsing(tr!("合成计划"), |ui| {
        let summary = app.synthesis.planner.summary();
        ui.label(trf!("{} 个音符组纳入当前计划", summary.pending_owners));
        ui.label(trf!(
            "本次变化 {} 组 · 保留 {} 组请求",
            summary.changed_owners,
            summary.unchanged_owners
        ));
        ui.weak(tr!(
            "优先准备播放位置的音频，其余部分在后台继续合成；无改动区段复用已有结果。"
        ));
    });
}
