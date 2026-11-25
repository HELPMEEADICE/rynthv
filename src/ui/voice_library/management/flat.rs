use super::super::{text, weak};
use crate::{
    app::App,
    synthesis::voice::{VoiceCatalogEntry, flat::ModelRole},
};
use eframe::egui::*;

pub(super) fn details(app: &mut App, ui: &mut Ui, entry: &VoiceCatalogEntry) {
    let Some(flat) = &entry.flat else { return };
    if let Some(version) = &flat.format_version {
        weak(ui, format!("Flat 格式版本 {version}"));
    }
    if let Some(inventory) = app.synthesis.inventory(&entry.selection()) {
        for warning in &inventory.flat_warnings {
            weak(ui, warning);
        }
    }
    ui.collapsing("Flat 模型依赖", |ui| {
        for dependency in &flat.dependencies {
            let role = match dependency.role {
                ModelRole::Acoustic => "基础声学模型",
                ModelRole::Vocoder => "声码器",
                ModelRole::Duration => "时长",
                ModelRole::F0 => "音高",
                ModelRole::SingingAssistant => "歌唱辅助",
            };
            let id = dependency.id.as_deref().unwrap_or("未声明");
            let status = if dependency.is_missing() {
                "缺失".to_owned()
            } else if dependency.role == ModelRole::SingingAssistant {
                "暂不支持".to_owned()
            } else {
                format!("找到 {} 个来源", dependency.sources.len())
            };
            text(ui, format!("{role} · {id} · {status}"));
        }
    });
    if app.voice_selection().as_ref() == Some(&entry.selection())
        && app.synthesis.inventory(&entry.selection()).is_some()
    {
        ui.collapsing("调整当前唱法", |ui| {
            super::super::parameters::styles(app, ui)
        });
    } else if !flat.styles.is_empty() {
        text(ui, format!("唱法 · {}", flat.styles.join(" · ")));
    }
}
