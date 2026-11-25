use crate::app::App;
use eframe::egui::*;

pub fn render_status(app: &mut App, ui: &mut Ui) {
    ui.label(RichText::new("歌声合成").strong());
    ui.separator();
    ui.label(app.synthesis.status());
    if let Some(progress) = &app.synthesis.progress {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(progress);
        });
    }
    let stats = app.synthesis.stats;
    ui.label(format!(
        "已完成 {} 组 · 处理中 {} 组 · 等待 {} 组 · 失败 {} 组",
        stats.ready, stats.running, stats.queued, stats.failed
    ));
    if let Some(error) = &app.synthesis.error {
        ui.colored_label(Color32::from_rgb(218, 142, 126), error);
    }
    for (selection, name) in app.synthesis.context_import_targets() {
        if ui.button(format!("为 {name} 导入模型上下文…")).clicked() {
            app.synthesis.open_context_import(selection);
        }
    }
    if !app.synthesis.failures.is_empty() {
        ui.collapsing("失败详情", |ui| {
            for (owner, error) in &app.synthesis.failures {
                ui.label(format!("音符组 {}：{error}", owner.0));
            }
        });
    }
    if (stats.failed > 0 || app.synthesis.error.is_some()) && ui.button("重试合成").clicked() {
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
            format!("播放设备：{error}"),
        );
    }
    ui.add_space(10.0);
    if let Some(duration) = app.synthesis.audio_duration() {
        ui.label(format!("当前混音 {:.2} 秒 · 48 kHz · 立体声", duration));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let (first, metrics) = app.synthesis.preview_timing();
        if first.is_some() || metrics.is_some() {
            ui.collapsing("预览耗时", |ui| {
                if let Some(first) = first {
                    ui.label(format!("请求至首个就绪 PCM 块 {first:.0} ms"));
                }
                if let Some(metrics) = metrics {
                    if let Some(first) = metrics.first_block_ms {
                        ui.label(format!("Worker 至首块 {first:.0} ms"));
                    }
                    ui.label(format!(
                        "Worker 完成 {:.0} ms · 区间混音 {:.0} ms",
                        metrics.worker_ms, metrics.mix_ms
                    ));
                    ui.label(format!(
                        "声库读取 {:.0} ms · 模型准备 {:.0} ms · 伴奏读取 {:.0} ms",
                        metrics.voice_load_ms, metrics.model_prepare_ms, metrics.recording_ms
                    ));
                    ui.label(format!(
                        "时长 {:.0} ms · 音高 {:.0} ms · 音色 {:.0} ms · 声码器 {:.0} ms",
                        metrics.stages.duration_ms,
                        metrics.stages.pitch_ms,
                        metrics.stages.timbre_ms,
                        metrics.stages.vocoder_ms
                    ));
                }
            });
        }
    }
    ui.collapsing("合成计划", |ui| {
        let summary = app.synthesis.planner.summary();
        ui.label(format!("{} 个音符组纳入当前计划", summary.pending_owners));
        ui.label(format!(
            "本次变化 {} 组 · 保留 {} 组请求",
            summary.changed_owners, summary.unchanged_owners
        ));
        ui.weak("优先准备播放位置的音频，其余部分在后台继续合成；无改动区段复用已有结果。");
    });
}
