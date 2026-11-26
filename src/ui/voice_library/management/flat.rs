use super::super::{text, weak};
use crate::{
    app::App,
    synthesis::voice::{VoiceCatalogEntry, flat::ModelRole},
    tr,
    trf,
};
use eframe::egui::*;

pub(super) fn details(app: &mut App, ui: &mut Ui, entry: &VoiceCatalogEntry) {
    let Some(flat) = &entry.flat else { return };
    if let Some(version) = &flat.format_version {
        weak(ui, trf!("Flat 格式版本 {}", version));
    }
    if let Some(inventory) = app.synthesis.inventory(&entry.selection()) {
        for warning in &inventory.flat_warnings {
            weak(ui, warning);
        }
    }
    ui.collapsing(tr!("Flat 模型依赖"), |ui| {
        for dependency in &flat.dependencies {
            let role = match dependency.role {
                ModelRole::Acoustic => tr!("基础声学模型"),
                ModelRole::Vocoder => tr!("声码器"),
                ModelRole::Duration => tr!("时长"),
                ModelRole::F0 => tr!("音高"),
                ModelRole::SingingAssistant => tr!("歌唱辅助"),
            };
            let id = dependency.id.as_deref().unwrap_or(tr!("未声明"));
            let status = if dependency.is_missing() {
                tr!("缺失").to_owned()
            } else if dependency.role == ModelRole::SingingAssistant {
                tr!("暂不支持").to_owned()
            } else {
                trf!("找到 {} 个来源", dependency.sources.len())
            };
            text(ui, format!("{role} · {id} · {status}"));
        }
    });
    if app.voice_selection().as_ref() == Some(&entry.selection())
        && app.synthesis.inventory(&entry.selection()).is_some()
    {
        ui.collapsing(tr!("调整当前唱法"), |ui| {
            super::super::parameters::styles(app, ui)
        });
    } else if !flat.styles.is_empty() {
        text(ui, trf!("唱法 · {}", flat.styles.join(" · ")));
    }
}
