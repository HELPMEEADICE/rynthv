use std::path::Path;

use super::{error_label, text, weak};
use crate::app::App;
use eframe::egui::*;

pub fn settings(app: &mut App, ui: &mut Ui) {
    text(ui, RichText::new("歌声资源").strong());
    text(ui, "SV2 声库根目录（databases）");
    path_label(ui, app.synthesis.library.settings.databases.as_deref());
    ui.horizontal(|ui| {
        if ui.button("选择 SV2 databases…").clicked() {
            choose_databases(app);
        }
        if ui
            .add_enabled(
                !app.synthesis.library.is_scanning()
                    && app.synthesis.library.settings.has_databases(),
                Button::new("重新扫描"),
            )
            .clicked()
        {
            app.synthesis.rescan_voices();
        }
    });
    ui.add_space(8.0);
    text(ui, "SV1 原生声库根目录（databases）");
    path_label(ui, app.synthesis.library.settings.sv1_databases.as_deref());
    if ui.button("选择 SV1 databases…").clicked() {
        choose_sv1_databases(app);
    }
    ui.add_space(8.0);
    text(ui, "SynthV Flat 声库目录");
    let flat_path = app.synthesis.library.settings.flat_voice_directory().ok();
    path_label(ui, flat_path.as_deref());
    ui.add_enabled_ui(!app.synthesis.library.flat_busy(), |ui| {
        if ui.button("选择 Flat 声库目录…").clicked() {
            choose_flat_databases(app);
        }
        ui.add_space(8.0);
        text(ui, "SynthV Flat Base Model 目录");
        let models = app.synthesis.library.settings.flat_models.as_deref();
        path_label(ui, models);
        ui.horizontal_wrapped(|ui| {
            if ui.button("选择 Base Model 目录…").clicked() {
                choose_flat_models(app);
            }
            if app.synthesis.library.settings.flat_models.is_some()
                && ui.button("清除目录").clicked()
            {
                app.synthesis.set_flat_models(None);
            }
        });
    });
    ui.add_space(8.0);
    text(ui, "发音词典目录（clf-data）");
    path_label(ui, app.synthesis.dictionaries());
    if ui.button("选择词典目录…").clicked() {
        choose_dictionaries(app);
    }
    if let Some(error) = app.synthesis.library.settings_error() {
        error_label(ui, error);
    }
    weak(
        ui,
        "请手动选择资源目录。路径和所选歌声会自动保存，下次启动恢复。",
    );
}

pub(super) fn choose_databases(app: &mut App) {
    if let Some(path) =
        folder_dialog(app.synthesis.library.settings.databases.as_deref()).pick_folder()
    {
        app.synthesis.set_databases(path);
    }
}

pub(super) fn choose_sv1_databases(app: &mut App) {
    if let Some(path) =
        folder_dialog(app.synthesis.library.settings.sv1_databases.as_deref()).pick_folder()
    {
        app.synthesis.set_sv1_databases(path);
    }
}

pub(super) fn directory_menu(app: &mut App, ui: &mut Ui) {
    if ui.button("SV2 声库目录…").clicked() {
        ui.close();
        choose_databases(app);
    }
    if ui.button("SV1 原生声库目录…").clicked() {
        ui.close();
        choose_sv1_databases(app);
    }
    ui.add_enabled_ui(!app.synthesis.library.flat_busy(), |ui| {
        if ui.button("SynthV Flat 声库目录…").clicked() {
            ui.close();
            choose_flat_databases(app);
        }
        if ui.button("SynthV Flat Base Model 目录…").clicked() {
            ui.close();
            choose_flat_models(app);
        }
    });
}

fn choose_flat_databases(app: &mut App) {
    let current = app.synthesis.library.settings.flat_voice_directory().ok();
    if let Some(path) = folder_dialog(current.as_deref()).pick_folder() {
        app.synthesis.set_flat_databases(path);
    }
}

fn choose_flat_models(app: &mut App) {
    if let Some(path) =
        folder_dialog(app.synthesis.library.settings.flat_models.as_deref()).pick_folder()
    {
        app.synthesis.set_flat_models(Some(path));
    }
}

pub(super) fn choose_dictionaries(app: &mut App) {
    if let Some(path) = folder_dialog(app.synthesis.dictionaries()).pick_folder() {
        app.synthesis.set_dictionaries(path);
    }
}

fn path_label(ui: &mut Ui, path: Option<&Path>) {
    match path {
        Some(path) => weak(ui, path.display().to_string()),
        None => weak(ui, "尚未指定"),
    }
}

fn folder_dialog(path: Option<&Path>) -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new();
    match path {
        Some(path) => dialog.set_directory(path),
        None => dialog,
    }
}
