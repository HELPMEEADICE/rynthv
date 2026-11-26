use super::{
    super::{context, family_name, language_name, text, weak},
    feedback, flat,
};
use crate::{
    app::App,
    synthesis::voice::{VoiceAvailability, VoiceCatalogEntry},
    trf,
};
use eframe::egui::*;

pub(super) fn show(app: &mut App, ui: &mut Ui, entry: &VoiceCatalogEntry) {
    ui.spacing_mut().item_spacing.y = 10.0;
    let selection = entry.selection();
    text(ui, trf!("声库类型：{}", family_name(entry)));
    if let Some(range) = &entry.pitch_range {
        weak(ui, trf!("推荐音域  {}", range));
    }
    if let Some(config) = app
        .synthesis
        .inventory(&selection)
        .and_then(|voice| voice.configuration.as_ref())
    {
        text(
            ui,
            trf!(
                "支持语言：{}",
                config
                    .supported_languages
                    .iter()
                    .map(|s| language_name(s))
                    .collect::<Vec<_>>()
                    .join("、")
            ),
        );
        if !config.styles.is_empty() {
            text(ui, trf!("唱法：{}", config.styles.join(" · ")));
        }
    } else if !entry.languages.is_empty() {
        text(
            ui,
            trf!(
                "语言：{}",
                entry
                    .languages
                    .iter()
                    .map(|s| language_name(s))
                    .collect::<Vec<_>>()
                    .join("、")
            ),
        );
    }
    if let VoiceAvailability::Unsupported(reason) = &entry.availability {
        super::super::error_label(ui, reason);
    }
    if entry.flat.is_some() {
        flat::details(app, ui, entry);
    }
    if app.synthesis.inventory(&selection).is_some()
        || app.synthesis.voice_error(&selection).is_some()
    {
        feedback::status(app, ui, &selection);
        if app.synthesis.voice_error(&selection).is_none() {
            context::controls(app, ui, &selection);
        }
    }
    feedback::diagnostics(app, ui, entry);
}
