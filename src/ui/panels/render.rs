use super::controls::*;
#[cfg(not(target_arch = "wasm32"))]
use crate::synthesis::runtime::ExportRequest;
use crate::{
    app::App,
    model::Extra,
    tr, trf,
    ui::{theme::*, track_color::TrackColors},
};
use eframe::egui::*;

pub(super) fn show(app: &mut App, ui: &mut Ui) {
    let mut pending_config: Vec<(&'static str, serde_json::Value)> = Vec::new();
    let mut pending_tracks: Vec<(u64, bool)> = Vec::new();
    let mut export_clicked = false;
    let editable = !app.editor.is_editing();

    padded(ui, |ui| {
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        let directory = directory_field(app, ui, editable);
        ui.weak(tr!("文件名称"));
        let name = string(&app.editor.project.render_config, "filename");
        let name_id = ui.make_persistent_id(("render-field", "filename"));
        let mut name_draft = ui
            .ctx()
            .data_mut(|d| d.get_temp::<RenderDraft>(name_id))
            .filter(|draft| draft.committed == name)
            .unwrap_or_else(|| RenderDraft::synced(&name));
        let name_response = field(ui, &mut name_draft.text);
        let name_commit = name_draft.text != name
            && (name_response.lost_focus()
                || name_response.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
        ui.add_space(12.0);
        let ready = !app.editor.is_editing() && app.synthesis.export_ready(app.editor.revision);
        export_clicked = ui
            .add_enabled(
                ready,
                Button::new(tr!("导出为文件")).min_size(vec2(ui.available_width(), 28.0)),
            )
            .on_disabled_hover_text(
                app.synthesis
                    .recording_error
                    .as_deref()
                    .unwrap_or(tr!("等待伴奏加载和当前歌声合成完成后导出")),
            )
            .clicked();
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        if let Some(path) = directory.commit(ui, editable, export_clicked) {
            pending_config.push(("destination", path.into()));
        }

        if editable && (name_commit || (export_clicked && name_draft.text != name)) {
            pending_config.push(("filename", name_draft.text.clone().into()));
            name_draft.committed = name_draft.text.clone();
        }
        if name_response.has_focus() || name_draft.text != name_draft.committed {
            ui.ctx().data_mut(|d| d.insert_temp(name_id, name_draft));
        } else {
            ui.ctx().data_mut(|d| d.remove::<RenderDraft>(name_id));
        }
        if app.synthesis.is_exporting() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr!("正在导出…"));
            });
        }
        #[cfg(target_os = "android")]
        if let Some(Ok(paths)) = app.synthesis.export_result.clone() {
            for path in paths {
                ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr!("保存 WAV…")).clicked() {
                        app.pick(
                            ui.ctx(),
                            crate::app::picker::Picker::publish(path.clone(), "audio/wav"),
                            |app, _, _| app.status = tr!("WAV 已保存").into(),
                        );
                    }
                    if ui.button(tr!("分享 WAV…")).clicked()
                        && let Err(error) = crate::platform::android::share(&path, "audio/wav")
                    {
                        app.error = Some(error);
                    }
                });
            }
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(Ok(paths)) = &app.synthesis.export_result {
            if let Some(metrics) = &app.synthesis.export_metrics {
                ui.weak(trf!(
                    "Worker {} ms · 渲染 {} ms · 混音 {} ms · WAV 转换和写入 {} ms",
                    format!("{:.0}", metrics.worker_ms),
                    format!("{:.0}", metrics.render_ms),
                    format!("{:.0}", metrics.mix_ms),
                    format!("{:.0}", metrics.wav_ms),
                ));
            }
            for path in paths {
                if ui
                    .button(trf!(
                        "下载 {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ))
                    .clicked()
                {
                    crate::app::web::download(path, ui.ctx());
                }
            }
        }
        if let Some(result) = &app.synthesis.export_result {
            match result {
                Ok(paths) => {
                    ui.label(trf!("已导出 {} 个文件", paths.len()))
                        .on_hover_text(
                            paths
                                .iter()
                                .map(|p| p.display().to_string())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        );
                }
                Err(message) => error(ui, message),
            }
        }
    });
    section(ui, tr!("音轨"), |ui| {
        let current_mix = app
            .editor
            .project
            .render_config
            .get("exportMixDown")
            .and_then(serde_json::Value::as_bool);
        let mut mix = current_mix.unwrap_or(true);
        let mut tracks: Vec<_> = app
            .editor
            .project
            .tracks
            .iter()
            .map(|t| {
                let current = t
                    .extra
                    .get("renderEnabled")
                    .and_then(serde_json::Value::as_bool);
                (
                    t.id,
                    t.name.clone(),
                    current,
                    current.unwrap_or(false),
                    TrackColors::new(t).accent,
                )
            })
            .collect();
        let mut mix_changed = false;
        Frame::new()
            .fill(background_color())
            .inner_margin(4)
            .show(ui, |ui| {
                ui.set_min_height(190.0);
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 3.0;
                if track_row(ui, &mut mix, tr!("混音导出"), text_color()) {
                    mix_changed = true;
                }
                for (id, name, _, enabled, color) in &mut tracks {
                    if track_row(ui, enabled, name, *color) {
                        pending_tracks.push((*id, *enabled));
                    }
                }
            });
        if mix_changed && current_mix != Some(mix) {
            pending_config.push(("exportMixDown", mix.into()));
        }
        pair(ui, |ui, width| {
            if ui
                .add_sized(vec2(width, 28.0), Button::new(tr!("全选")))
                .clicked()
            {
                if current_mix != Some(true) {
                    pending_config.push(("exportMixDown", true.into()));
                }
                for (id, _, current, _, _) in &tracks {
                    if *current != Some(true) {
                        pending_tracks.push((*id, true));
                    }
                }
            }
            if ui
                .add_sized(vec2(width, 28.0), Button::new(tr!("取消全选")))
                .clicked()
            {
                if current_mix != Some(false) {
                    pending_config.push(("exportMixDown", false.into()));
                }
                for (id, _, current, _, _) in &tracks {
                    if *current != Some(false) {
                        pending_tracks.push((*id, false));
                    }
                }
            }
        });
    });
    section(ui, tr!("格式"), |ui| {
        if let Some((key, value)) = numeric(
            ui,
            &app.editor.project.render_config,
            "numChannels",
            tr!("声道"),
            &[(1, tr!("单声道")), (2, tr!("立体声"))],
        ) {
            pending_config.push((key, value.into()));
        }
        ui.weak(tr!("气声输出"));
        let mut aspiration = string(&app.editor.project.render_config, "aspirationFormat");
        if combo(
            ui,
            "aspiration-format",
            &mut aspiration,
            &[
                ("noAspiration".to_owned(), tr!("与人声合并")),
                ("asFiles".to_owned(), tr!("单独导出气声文件")),
            ],
        ) {
            pending_config.push(("aspirationFormat", aspiration.into()));
        }
        if let Some((key, value)) = numeric(
            ui,
            &app.editor.project.render_config,
            "bitDepth",
            tr!("位深度"),
            &[(16, tr!("16 位")), (24, tr!("24 位")), (32, tr!("32 位浮点"))],
        ) {
            pending_config.push((key, value.into()));
        }
        if let Some((key, value)) = numeric(
            ui,
            &app.editor.project.render_config,
            "sampleRate",
            tr!("采样率"),
            &[
                (44100, "44100 Hz"),
                (48000, "48000 Hz"),
                (96000, "96000 Hz"),
            ],
        ) {
            pending_config.push((key, value.into()));
        }
    });
    if editable && (!pending_config.is_empty() || !pending_tracks.is_empty()) {
        app.editor.begin(tr!("渲染设置"));
        for (key, value) in pending_config {
            app.editor.project.render_config.insert(key.into(), value);
        }
        for (id, enabled) in pending_tracks {
            if let Some(track) = app.editor.project.tracks.iter_mut().find(|t| t.id == id) {
                track.extra.insert("renderEnabled".into(), enabled.into());
            }
        }
        app.editor.finish();
    }
    if export_clicked {
        start_export(app);
    }
    section(ui, tr!("合成状态"), |ui| {
        super::super::synthesis::render_status(app, ui)
    });
}

#[derive(Clone)]
struct RenderDraft {
    committed: String,
    text: String,
}

impl RenderDraft {
    fn synced(value: &str) -> Self {
        Self {
            committed: value.to_owned(),
            text: value.to_owned(),
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn start_export(app: &mut App) {
    app.synthesis
        .sync_project(&app.editor.project, app.editor.revision);
    app.synthesis.export_project(app.editor.revision);
}

#[cfg(not(target_arch = "wasm32"))]
fn start_export(app: &mut App) {
    #[cfg(not(target_os = "android"))]
    let request = ExportRequest::from_project(&app.editor.project);
    #[cfg(target_os = "android")]
    let request = crate::platform::android::cache_directory().and_then(|root| {
        ExportRequest::with_directory(
            &app.editor.project,
            Some(root.join("renders").join(uuid::Uuid::new_v4().to_string())),
        )
    });
    let request = match request {
        Ok(request) => request,
        Err(message) => {
            app.synthesis.export_result = Some(Err(message));
            return;
        }
    };
    #[cfg(not(target_os = "android"))]
    if !confirm_overwrite(&request) {
        return;
    }
    app.synthesis
        .sync_project(&app.editor.project, app.editor.revision);
    app.synthesis.export_project(request, app.editor.revision);
}

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
fn confirm_overwrite(request: &ExportRequest) -> bool {
    let existing: Vec<_> = request
        .paths()
        .filter(|path| path.exists())
        .map(|path| path.display().to_string())
        .collect();
    existing.is_empty()
        || rfd::MessageDialog::new()
            .set_title(tr!("覆盖音频文件"))
            .set_description(trf!(
                "以下文件已存在，是否覆盖？\n{}",
                existing.join("\n")
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes
}

fn string(config: &Extra, key: &str) -> String {
    config
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned()
}
fn numeric(
    ui: &mut Ui,
    config: &Extra,
    key: &'static str,
    label: &str,
    values: &[(u64, &str)],
) -> Option<(&'static str, u64)> {
    ui.weak(label);
    let mut value = config
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(values[0].0);
    combo(ui, key, &mut value, values).then_some((key, value))
}
fn track_row(ui: &mut Ui, checked: &mut bool, name: &str, color: Color32) -> bool {
    let row = Frame::new()
        .fill(panel_color())
        .corner_radius(2)
        .inner_margin(Margin {
            left: 12,
            right: 6,
            top: 5,
            bottom: 5,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.checkbox(checked, name).changed()
        });
    ui.painter().rect_filled(
        Rect::from_min_size(row.response.rect.min, vec2(3.0, row.response.rect.height())),
        0,
        color,
    );
    row.inner
}
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
fn folder_button(ui: &mut Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    ui.painter().rect(
        rect,
        3,
        if response.hovered() {
            button_hover_color()
        } else {
            border_color()
        },
        Stroke::new(0.6, button_color()),
        StrokeKind::Inside,
    );
    let c = rect.center();
    let points = [
        (-5.5, -3.0),
        (-5.5, -5.0),
        (-1.0, -5.0),
        (1.0, -3.0),
        (5.5, -3.0),
        (5.5, 5.0),
        (-5.5, 5.0),
        (-5.5, -3.0),
    ];
    ui.painter().add(Shape::line(
        points.into_iter().map(|(x, y)| c + vec2(x, y)).collect(),
        Stroke::new(1.1, muted_color()),
    ));
    response.on_hover_text(tr!("选择目标文件夹"))
}

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
struct DirectoryField {
    id: Id,
    original: String,
    draft: RenderDraft,
    has_focus: bool,
    changed: bool,
}

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
impl DirectoryField {
    fn commit(self, ui: &Ui, editable: bool, export_clicked: bool) -> Option<String> {
        let Self {
            id,
            original,
            mut draft,
            has_focus,
            changed,
        } = self;
        let saved = (editable && (changed || (export_clicked && draft.text != original)))
            .then(|| draft.text.clone());
        if let Some(path) = &saved {
            draft.committed.clone_from(path);
        }
        if has_focus || draft.text != draft.committed {
            ui.ctx().data_mut(|d| d.insert_temp(id, draft));
        } else {
            ui.ctx().data_mut(|d| d.remove::<RenderDraft>(id));
        }
        saved
    }
}

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
fn directory_field(app: &App, ui: &mut Ui, editable: bool) -> DirectoryField {
    ui.weak(tr!("目标文件夹"));
    let directory = string(&app.editor.project.render_config, "destination");
    let dir_id = ui.make_persistent_id(("render-field", "destination"));
    let mut dir_draft = ui
        .ctx()
        .data_mut(|d| d.get_temp::<RenderDraft>(dir_id))
        .filter(|draft| draft.committed == directory)
        .unwrap_or_else(|| RenderDraft::synced(&directory));
    let mut dir_commit = false;
    let mut dir_has_focus = false;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 34.0).max(20.0);
        let response = ui.add_sized(
            vec2(width, 28.0),
            TextEdit::singleline(&mut dir_draft.text)
                .vertical_align(Align::Center)
                .margin(vec2(7.0, 5.0))
                .background_color(border_color()),
        );
        dir_has_focus = response.has_focus();
        if dir_draft.text != directory
            && (response.lost_focus() || dir_has_focus && ui.input(|i| i.key_pressed(Key::Enter)))
        {
            dir_commit = true;
        }
        if folder_button(ui).clicked() && editable {
            let mut picker = rfd::FileDialog::new().set_title(tr!("选择导出文件夹"));
            if !dir_draft.text.is_empty() {
                picker = picker.set_directory(&dir_draft.text);
            }
            if let Some(path) = picker.pick_folder() {
                dir_draft.text = path.display().to_string();
                dir_commit = true;
            }
        }
    });
    DirectoryField {
        id: dir_id,
        original: directory,
        draft: dir_draft,
        has_focus: dir_has_focus,
        changed: dir_commit,
    }
}
