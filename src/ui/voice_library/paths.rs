use std::path::Path;

use super::{error_label, resource_import, text, weak};
use crate::app::{
    App,
    picker::{Picker, Selection},
};
use crate::tr;
use eframe::egui::*;

pub fn settings(app: &mut App, ui: &mut Ui) {
    text(ui, RichText::new(tr!("歌声资源")).strong());
    resource_import::voices(app, ui);
    text(ui, tr!("SV2 声库根目录（databases）"));
    path_label(ui, app.synthesis.library.settings.databases.as_deref());
    ui.horizontal(|ui| {
        if ui.button(tr!("选择 SV2 databases…")).clicked() {
            choose_databases(app, ui.ctx());
        }
        if ui
            .add_enabled(
                !app.synthesis.library.is_scanning() && app.synthesis.library.has_databases(),
                Button::new(tr!("重新扫描")),
            )
            .clicked()
        {
            app.synthesis.rescan_voices();
        }
    });
    ui.add_space(8.0);
    text(ui, tr!("SV1 原生声库根目录（databases）"));
    path_label(ui, app.synthesis.library.settings.sv1_databases.as_deref());
    if ui.button(tr!("选择 SV1 databases…")).clicked() {
        choose_sv1_databases(app, ui.ctx());
    }
    ui.add_space(8.0);
    text(ui, tr!("SynthV Flat 声库目录"));
    let flat_path = app.synthesis.library.settings.flat_voice_directory().ok();
    path_label(ui, flat_path.as_deref());
    ui.add_enabled_ui(!app.synthesis.library.resource_busy(), |ui| {
        if ui.button(tr!("选择 Flat 声库目录…")).clicked() {
            choose_flat_databases(app, ui.ctx());
        }
        ui.add_space(8.0);
        text(ui, tr!("SynthV Flat Base Model 目录"));
        let models = app.synthesis.library.settings.flat_models.as_deref();
        path_label(ui, models);
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr!("选择 Base Model 目录…")).clicked() {
                choose_flat_models(app, ui.ctx());
            }
            if app.synthesis.library.settings.flat_models.is_some()
                && ui.button(tr!("清除目录")).clicked()
            {
                app.synthesis.set_flat_models(None);
            }
        });
    });
    ui.add_space(8.0);
    text(ui, tr!("发音词典目录（clf-data）"));
    path_label(ui, app.synthesis.dictionaries());
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                !app.synthesis.library.resource_busy(),
                Button::new(tr!("选择词典目录…")),
            )
            .clicked()
        {
            choose_dictionaries(app, ui.ctx());
        }
        resource_import::dictionaries(app, ui);
    });
    resource_import::status(app, ui);
    #[cfg(target_arch = "wasm32")]
    app.browser_storage_panel(ui);
    if let Some(error) = app.synthesis.library.settings_error() {
        error_label(ui, error);
    }
    weak(ui, tr!("资源路径和所选歌声会自动保存，下次启动恢复。"));
}

pub(super) fn choose_databases(app: &mut App, ctx: &Context) {
    let picker = Picker::new(Selection::Directory, tr!("选择 SV2 databases"))
        .starting_at(app.synthesis.library.settings.databases.as_deref());
    app.pick_one(ctx, picker, |app, _, path| {
        app.synthesis.set_databases(path)
    });
}

pub(super) fn choose_sv1_databases(app: &mut App, ctx: &Context) {
    let picker = Picker::new(Selection::Directory, tr!("选择 SV1 databases"))
        .starting_at(app.synthesis.library.settings.sv1_databases.as_deref());
    app.pick_one(ctx, picker, |app, _, path| {
        app.synthesis.set_sv1_databases(path)
    });
}

pub(super) fn directory_menu(app: &mut App, ui: &mut Ui) {
    if ui.button(tr!("SV2 声库目录…")).clicked() {
        ui.close();
        choose_databases(app, ui.ctx());
    }
    if ui.button(tr!("SV1 原生声库目录…")).clicked() {
        ui.close();
        choose_sv1_databases(app, ui.ctx());
    }
    ui.add_enabled_ui(!app.synthesis.library.resource_busy(), |ui| {
        if ui.button(tr!("SynthV Flat 声库目录…")).clicked() {
            ui.close();
            choose_flat_databases(app, ui.ctx());
        }
        if ui.button(tr!("SynthV Flat Base Model 目录…")).clicked() {
            ui.close();
            choose_flat_models(app, ui.ctx());
        }
    });
}

fn choose_flat_databases(app: &mut App, ctx: &Context) {
    let current = app.synthesis.library.settings.flat_voice_directory().ok();
    let picker = Picker::new(Selection::Directory, tr!("选择 Flat 声库目录"))
        .starting_at(current.as_deref());
    app.pick_one(ctx, picker, |app, _, path| {
        app.synthesis.set_flat_databases(path)
    });
}

fn choose_flat_models(app: &mut App, ctx: &Context) {
    let picker = Picker::new(Selection::Directory, tr!("选择 Base Model 目录"))
        .starting_at(app.synthesis.library.settings.flat_models.as_deref());
    app.pick_one(ctx, picker, |app, _, path| {
        app.synthesis.set_flat_models(Some(path))
    });
}

pub(super) fn choose_dictionaries(app: &mut App, ctx: &Context) {
    let picker = Picker::new(Selection::Directory, tr!("选择词典目录"))
        .starting_at(app.synthesis.dictionaries());
    app.pick_one(ctx, picker, |app, _, path| {
        app.synthesis.set_dictionaries(path)
    });
}

fn path_label(ui: &mut Ui, path: Option<&Path>) {
    match path {
        Some(path) => weak(ui, path.display().to_string()),
        None => weak(ui, tr!("尚未指定")),
    }
}
