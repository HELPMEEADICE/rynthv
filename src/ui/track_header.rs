mod source;

use super::{icons::Icon, theme::*};
use crate::{app::App, tr, trf};
use eframe::egui::*;

pub fn show(app: &mut App, ui: &mut Ui, rect: Rect, track_index: usize) {
    let (track_id, track_name, track_color, is_instrumental, mixer_values, mute_solo) = {
        let track = &app.editor.project.tracks[track_index];
        (
            track.id,
            track.name.clone(),
            track.disp_color,
            track.is_instrumental(),
            [
                track
                    .mixer
                    .get("gainDecibel")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0),
                track
                    .mixer
                    .get("pan")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0),
            ],
            [
                track
                    .mixer
                    .get("mute")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                track
                    .mixer
                    .get("solo")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            ],
        )
    };
    let track_count = app.editor.project.tracks.len();
    let response = ui.interact(rect, Id::new(("track", track_id)), Sense::click());
    if response.clicked() && !is_instrumental {
        app.commit_lyric();
        app.editor.select_track(track_id);
    }
    if response.double_clicked() {
        app.rename = Some(super::rename::Rename::track(track_id, &track_name));
    }
    response.context_menu(|ui| {
        if ui.button(tr!("重命名音轨…")).clicked() {
            app.rename = Some(super::rename::Rename::track(track_id, &track_name));
            ui.close();
        }
        if ui.button(tr!("新增音轨")).clicked() {
            app.editor.add_track();
            ui.close();
        }
        if ui.button(tr!("新增伴奏轨…")).clicked() {
            let id = app.editor.add_instrumental_track();
            ui.close();
            app.import_audio(Some(id), ui.ctx());
        }
        if ui
            .add_enabled(track_index > 0, Button::new(tr!("上移音轨")))
            .clicked()
        {
            app.editor.reorder_track(track_id, -1);
            ui.close();
        }
        if ui
            .add_enabled(track_index + 1 < track_count, Button::new(tr!("下移音轨")))
            .clicked()
        {
            app.editor.reorder_track(track_id, 1);
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(track_count > 1, Button::new(tr!("删除音轨")))
            .clicked()
        {
            app.commit_lyric();
            app.editor.delete_track(track_id);
            ui.close();
        }
        ui.separator();
        if let Some(color) = super::track_color::picker(ui, track_color) {
            app.commit_lyric();
            app.editor.set_track_color(track_id, color);
            ui.close();
        }
    });
    let mixer_width = if cfg!(target_os = "android") && rect.width() < 210.0 {
        6.0
    } else {
        95.0
    };
    let text_rect = Rect::from_min_max(
        rect.min + vec2(16.0, 0.0),
        pos2(rect.right() - mixer_width, rect.bottom()),
    );
    let painter = ui
        .painter()
        .with_clip_rect(ui.clip_rect().intersect(text_rect));
    painter.text(
        rect.min + vec2(18.0, 17.0),
        Align2::LEFT_CENTER,
        &track_name,
        FontId::proportional(12.0),
        text_color(),
    );
    source::show(app, ui, rect, text_rect, track_index);
    if rect.width() < 210.0 {
        return;
    }
    for (idx, (field, label, center, min, max, reset)) in [
        (
            "gainDecibel",
            tr!("音量"),
            pos2(rect.right() - 79.0, rect.top() + 23.0),
            -48.0,
            12.0,
            0.0,
        ),
        (
            "pan",
            tr!("声像"),
            pos2(rect.right() - 49.0, rect.top() + 23.0),
            -1.0,
            1.0,
            0.0,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let old = mixer_values[idx];
        let (value, response) = knob(ui, (track_id, field), center, old, min, max, reset);
        let response = response.on_hover_ui(|ui| {
            ui.label(trf!(
                "{}  {}\n拖动调整，双击复位",
                label,
                format!("{old:.2}")
            ));
        });
        let frame = ui.ctx().cumulative_frame_nr();
        if response.double_clicked_by(PointerButton::Primary) && !app.editor.is_editing() {
            app.commit_lyric();
            app.editor.begin(tr!("调整混音"));
            if let Some(track) = app
                .editor
                .project
                .tracks
                .iter_mut()
                .find(|t| t.id == track_id)
            {
                track.mixer.insert(field.into(), reset.into());
            }
            app.editor.finish();
        } else {
            if response.drag_started_by(PointerButton::Primary) && !app.editor.is_editing() {
                app.commit_lyric();
                app.editor.begin(tr!("调整混音"));
                app.mixer_drag = Some((track_id, field, frame));
            }
            if let Some((drag_track, drag_field, drag_frame)) = app.mixer_drag.as_mut()
                && *drag_track == track_id
                && *drag_field == field
            {
                *drag_frame = frame;
                if value != old
                    && let Some(track) = app
                        .editor
                        .project
                        .tracks
                        .iter_mut()
                        .find(|t| t.id == track_id)
                {
                    track.mixer.insert(field.into(), value.into());
                }
                if response.drag_stopped_by(PointerButton::Primary) {
                    app.editor.finish();
                    app.mixer_drag = None;
                }
            }
        }
        if field == "pan" {
            Icon::Pan.paint(
                ui,
                Rect::from_center_size(pos2(center.x, rect.top() + 43.0), vec2(13.0, 13.0)),
                Color32::from_rgb(120, 117, 124),
            );
        } else {
            let x = center.x - 6.0;
            for i in 0..6 {
                ui.painter().vline(
                    x + i as f32 * 2.0,
                    rect.top() + 45.0 - (1.0 + i as f32)..=rect.top() + 45.0,
                    Stroke::new(1.0, muted_color().gamma_multiply(0.55)),
                );
            }
        }
    }
    for (idx, (field, text, y)) in [("mute", "M", 10.0), ("solo", "S", 30.0)]
        .into_iter()
        .enumerate()
    {
        let value = mute_solo[idx];
        let r = Rect::from_min_size(pos2(rect.right() - 27.0, rect.top() + y), vec2(16.0, 16.0));
        let response = ui.interact(r, Id::new((track_id, field)), Sense::click());
        ui.painter().rect_filled(
            r,
            2,
            if value {
                green_color()
            } else if response.hovered() {
                button_hover_color()
            } else {
                button_color()
            },
        );
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(9.0),
            if value { canvas_color() } else { text_color() },
        );
        if response
            .on_hover_text(if field == "mute" {
                tr!("音轨静音")
            } else {
                tr!("音轨独奏")
            })
            .clicked()
        {
            app.editor.begin(tr!("音轨混音"));
            if let Some(track) = app
                .editor
                .project
                .tracks
                .iter_mut()
                .find(|t| t.id == track_id)
            {
                track.mixer.insert(field.into(), (!value).into());
            }
            app.editor.finish();
        }
    }
}
fn knob(
    ui: &mut Ui,
    id: (u64, &str),
    center: Pos2,
    value: f64,
    min: f64,
    max: f64,
    reset: f64,
) -> (f64, Response) {
    let rect = Rect::from_center_size(center, vec2(21.0, 25.0));
    let response = ui.interact(rect, Id::new(id), Sense::click_and_drag());
    ui.painter().circle_filled(
        center,
        9.5,
        if response.hovered() {
            button_hover_color()
        } else {
            Color32::from_rgb(77, 75, 81)
        },
    );
    let normalized = if value <= reset {
        ((value - reset) / (reset - min).max(1e-6)) * 0.5 + 0.5
    } else {
        ((value - reset) / (max - reset).max(1e-6)) * 0.5 + 0.5
    };
    let angle = (-225.0 + normalized as f32 * 270.0).to_radians();
    let end = center + vec2(angle.cos(), angle.sin()) * 8.0;
    ui.painter()
        .line_segment([center, end], Stroke::new(1.0, muted_color()));
    let next = if response.double_clicked_by(PointerButton::Primary) {
        reset
    } else if response.dragged_by(PointerButton::Primary) {
        (value - response.drag_delta().y as f64 * (max - min) / 160.0).clamp(min, max)
    } else {
        value
    };
    (next, response)
}

pub fn end_frame(app: &mut App, ctx: &Context) {
    let Some((_, _, frame)) = app.mixer_drag else {
        return;
    };
    let inactive = frame != ctx.cumulative_frame_nr()
        || app.pending.is_some()
        || app.error.is_some()
        || ctx.input(|input| !input.focused || !input.pointer.primary_down());
    if inactive {
        app.editor.finish();
        app.mixer_drag = None;
    }
}
