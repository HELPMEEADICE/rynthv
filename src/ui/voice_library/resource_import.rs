use super::{error_label, weak};
use crate::app::{
    App,
    picker::{Picker, Selection},
};
use eframe::egui::*;

pub(super) fn voices(app: &mut App, ui: &mut Ui) {
    if ui
        .add_enabled(
            available(app),
            Button::new("导入声库…").min_size(vec2(98.0, 36.0)),
        )
        .clicked()
    {
        app.pick(
            ui.ctx(),
            Picker::new(Selection::Files, "导入声库")
                .filter("SynthV 声库包", &["svpk", "sfpk", "sfpks"]),
            |app, _, paths| app.synthesis.import_voices(paths),
        );
    }
}

pub(super) fn dictionaries(app: &mut App, ui: &mut Ui) {
    if ui
        .add_enabled(available(app), Button::new("导入词典压缩包…"))
        .clicked()
    {
        app.pick_one(
            ui.ctx(),
            Picker::new(Selection::File, "导入 clf-data 词典").filter("ZIP / 7z", &["zip", "7z"]),
            |app, _, path| app.synthesis.import_dictionaries(path),
        );
    }
}

pub(super) fn status(app: &App, ui: &mut Ui) {
    let library = &app.synthesis.library;
    if library.resource_busy() {
        ui.horizontal(|ui| {
            ui.spinner();
            weak(ui, "正在处理歌声资源…");
        });
    } else if let Some(result) = library.resource_feedback() {
        match result {
            Ok(message) => weak(ui, message),
            Err(error) => error_label(ui, error),
        }
    }
}

fn available(app: &App) -> bool {
    let library = &app.synthesis.library;
    !library.resource_busy() && library.settings_error().is_none()
}
