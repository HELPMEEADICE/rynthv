use super::{
    Gesture,
    gestures::{pitch_at, start_gesture, update_gesture},
    scene,
};
use crate::{
    app::App,
    model::*,
    tr,
    ui::{theme::*, timeline::*},
};
use eframe::egui::*;
use std::collections::BTreeSet;

pub fn note_rect(note: &Note, offset: i64, pitch_offset: i32, rect: Rect, app: &App) -> Rect {
    let x = x_at(rect, &app.view.piano, beats(note.onset + offset));
    let y = rect.top()
        + (app.view.top_pitch - (note.pitch + pitch_offset) as f32) * app.view.row_height;
    Rect::from_min_size(
        pos2(x, y + 1.0),
        vec2(
            (beats(note.duration) * app.view.piano.beat_width as f64) as f32,
            app.view.row_height - 2.0,
        ),
    )
}
pub fn show(app: &mut App, ui: &mut Ui) {
    let all = ui.max_rect();
    if ui.rect_contains_pointer(all) && ui.input(|input| input.pointer.any_pressed()) {
        crate::ui::lanes::parameter_deactivate(app);
        crate::ui::phonemes::phoneme_deactivate(app);
    }
    let phoneme_dragging = crate::ui::phonemes::phoneme_dragging(app);
    ui.painter().rect_filled(all, 0, panel_color());
    let ruler_height = if cfg!(target_os = "android") {
        26.0
    } else {
        38.0
    };
    let ruler = Rect::from_min_max(
        pos2(all.left() + GUTTER, all.top()),
        pos2(all.right(), all.top() + ruler_height),
    );
    let canvas = Rect::from_min_max(
        pos2(ruler.left(), ruler.bottom() + super::GROUP_LABEL_HEIGHT),
        all.max - vec2(0.0, 6.0),
    );
    if canvas.height() < 8.0 || canvas.width() < 18.0 {
        return;
    }
    #[cfg(any(target_os = "android", target_arch = "wasm32"))]
    let touch_transforming = super::touch::transform(app, ui, canvas);
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    let touch_transforming = false;
    let pan = app.view_pan.drag(
        ui,
        crate::ui::view_pan::Target::Piano,
        Rect::from_min_max(all.min, canvas.max - vec2(6.0, 0.0)),
        app.gesture.is_none()
            && app.group_gesture.is_none()
            && !phoneme_dragging
            && !touch_transforming,
    );
    app.view.piano.start =
        (app.view.piano.start - pan.x as f64 / app.view.piano.beat_width as f64).max(0.0);
    let response = ui.interact(canvas, Id::new("piano-canvas"), Sense::click_and_drag());
    let vertical = if touch_transforming
        || phoneme_dragging
        || matches!(app.gesture, Some(Gesture::Pitch(_) | Gesture::Anchor(_)))
    {
        0.0
    } else if app.view_pan.is_active() {
        pan.y
    } else {
        scroll(ui, &response, canvas, &mut app.view.piano)
    };
    app.view.top_pitch = (app.view.top_pitch + vertical / app.view.row_height).clamp(12.0, 127.0);
    follow_playhead(
        ui,
        Rect::from_min_max(ruler.min, canvas.max),
        &mut app.view.piano,
        app.editor.transport.position,
        app.editor.transport.playing
            && app.gesture.is_none()
            && app.group_gesture.is_none()
            && app.audio_drag.is_none()
            && !phoneme_dragging
            && !app.view_pan.is_active()
            && !touch_transforming,
    );
    super::background::draw(app, ui, canvas);
    super::keyboard::show(app, ui, all, canvas);
    super::scale_guide::draw_tonic_lines(app, ui, canvas);
    grid(
        ui,
        Rect::from_min_max(pos2(ruler.left(), ruler.bottom()), canvas.max),
        &app.view.piano,
        &app.editor.project.time,
        app.view.step(),
    );
    loop_region::show(
        ui,
        Id::new("piano-ruler"),
        ruler,
        &app.view.piano,
        &mut app.editor.transport,
        loop_region::RulerOptions {
            marker_step: app.view.snap.then_some(app.view.step()),
            seek_step: app.view.snap.then_some(app.view.step()),
            enabled: !touch_transforming
                && !app.view_pan.is_active()
                && app.gesture.is_none()
                && app.group_gesture.is_none()
                && app.audio_drag.is_none()
                && !phoneme_dragging,
        },
    );
    draw_ruler(
        ui,
        ruler,
        &app.view.piano,
        &app.editor.project.time,
        app.view.step(),
    );
    let groups = scene::snapshot(app, canvas);
    let mut changed_scene = false;
    if !touch_transforming
        && !app.editor.current_track().is_instrumental()
        && app.group_gesture.is_none()
        && !app.view_pan.is_active()
        && !phoneme_dragging
    {
        let before = (
            app.editor.revision,
            app.editor.reference,
            app.editor.selected.len(),
            app.editor.is_editing(),
        );
        if app.piano_tool == super::Tool::PitchDraw {
            super::pitch_edit::interaction(app, ui, &response, canvas);
        } else if app.piano_tool == super::Tool::PitchAnchor {
            super::pitch_anchor::interaction(app, ui, &response, canvas);
        } else {
            interaction(app, ui, &response, canvas, &groups);
        }
        changed_scene = before
            != (
                app.editor.revision,
                app.editor.reference,
                app.editor.selected.len(),
                app.editor.is_editing(),
            )
            || app.gesture.is_some();
    }
    let groups = if changed_scene {
        scene::snapshot(app, canvas)
    } else {
        groups
    };
    super::background_notes::draw(app, ui, canvas);
    super::rendered::waveforms(app, ui, canvas, &groups);
    scene::draw_notes(app, ui, canvas, &groups);
    super::rendered::phonemes(app, ui, canvas, &groups);
    super::rendered::pitch(app, ui, canvas, &groups);
    super::pitch_edit::draw(app, ui, canvas, &groups);
    super::pitch_anchor::draw(app, ui, canvas);
    super::retakes::show(app, ui, canvas, &groups);
    scene::headers(app, ui, canvas, &groups);
    if let Some(Gesture::Select { origin, .. }) = &app.gesture
        && let Some(p) = ui.input(|i| i.pointer.interact_pos())
    {
        let r = Rect::from_two_pos(*origin, p).intersect(canvas);
        ui.painter()
            .rect_filled(r, 0, green_color().gamma_multiply(0.12));
        ui.painter()
            .rect_stroke(r, 0, Stroke::new(1.0, green_color()), StrokeKind::Inside);
    }
    playhead(
        ui,
        Rect::from_min_max(ruler.min, canvas.max),
        ruler.bottom(),
        &app.view.piano,
        app.editor.transport.position,
    );
    #[cfg(not(target_os = "android"))]
    if !app.piano_tool.is_pitch()
        && let Some(reference) = app.editor.active_ref().cloned()
    {
        super::lyrics::lyric(app, ui, canvas, &reference);
    }
    crate::ui::scrollbars::horizontal(
        ui,
        "piano-scrollbar",
        Rect::from_min_max(
            pos2(canvas.left() + 2.0, all.bottom() - 6.0),
            all.max - vec2(8.0, 0.0),
        ),
        &mut app.view.piano,
        beats(app.editor.project_end()) + 32.0,
    );
    crate::ui::scrollbars::vertical(
        ui,
        Rect::from_min_max(pos2(canvas.right() - 6.0, canvas.top()), canvas.max),
        &mut app.view.top_pitch,
        app.view.row_height,
    );
}
fn interaction(
    app: &mut App,
    ui: &mut Ui,
    response: &Response,
    canvas: Rect,
    groups: &[scene::SceneGroup],
) {
    let hit = response
        .interact_pointer_pos()
        .and_then(|p| scene::hit_note(app, groups, canvas, p));
    if let Some(p) = ui.input(|i| i.pointer.hover_pos())
        && let Some((r, n)) = scene::hit_note(app, groups, canvas, p)
    {
        let rect = note_rect(&n, r.blick_offset, r.pitch_offset, canvas, app);
        if (p.x - rect.left()).abs() < 6.0 || (p.x - rect.right()).abs() < 6.0 {
            response
                .clone()
                .on_hover_cursor(CursorIcon::ResizeHorizontal);
        }
    }
    if response.double_clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        app.commit_lyric();
        if let Some((reference, note)) = hit.as_ref() {
            app.editor.select_reference(app.editor.track, reference.id);
            app.editor.selected = BTreeSet::from([note.id]);
            app.edit_lyric();
        } else {
            app.editor.begin(tr!("创建音符"));
            let at = ticks(app.view.snap_beat(beat_at(canvas, &app.view.piano, p.x))).max(0);
            if app
                .editor
                .add_note(at, QUARTER, pitch_at(app, canvas, p.y))
                .is_none()
            {
                app.status = tr!("请在音符组范围内创建音符，或在编排中插入新的音符组").into();
            }
            app.editor.finish();
        }
    } else if response.clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        app.commit_lyric();
        if let Some((reference, note)) = hit.as_ref() {
            app.editor.select_reference(app.editor.track, reference.id);
            if ui.input(|i| i.modifiers.ctrl || i.modifiers.shift) {
                if !app.editor.selected.remove(&note.id) {
                    app.editor.selected.insert(note.id);
                }
            } else {
                app.editor.selected = BTreeSet::from([note.id]);
            }
        } else if (cfg!(target_os = "android")
            || cfg!(target_arch = "wasm32") && ui.input(|input| input.has_touch_screen()))
            && app.piano_tool == super::Tool::NoteDraw
        {
            app.editor.begin(tr!("创建音符"));
            let at = ticks(app.view.snap_beat(beat_at(canvas, &app.view.piano, p.x))).max(0);
            let duration = ticks(app.view.step());
            app.editor
                .add_note(at, duration, pitch_at(app, canvas, p.y));
            app.editor.finish();
        } else {
            let at = ticks(beat_at(canvas, &app.view.piano, p.x));
            if let Some(id) = app.editor.group_at(at) {
                app.editor.select_reference(app.editor.track, id);
            } else {
                app.editor.select_track(app.editor.track);
            }
            app.editor.selected.clear();
        }
    }
    if response.secondary_clicked()
        && let Some(p) = response.interact_pointer_pos()
        && let Some((reference, note)) = scene::hit_note(app, groups, canvas, p)
    {
        app.commit_lyric();
        app.editor.select_reference(app.editor.track, reference.id);
        if !app.editor.selected.contains(&note.id) && !response.long_touched() {
            app.editor.selected = BTreeSet::from([note.id]);
        } else if app.editor.selected.is_empty() {
            app.editor.selected.insert(note.id);
        }
        #[cfg(target_os = "android")]
        ui.ctx()
            .data_mut(|data| data.insert_temp(response.id.with("note-context"), note.id));
    }
    response.context_menu(|ui| {
        let selected = !app.editor.selected.is_empty();
        #[cfg(target_os = "android")]
        if let Some(note) = ui
            .ctx()
            .data(|data| data.get_temp::<EntityId>(response.id.with("note-context")))
        {
            let was_selected = app.editor.selected.contains(&note);
            let label = if was_selected {
                tr!("移出选择")
            } else {
                tr!("加入选择")
            };
            if ui.button(label).clicked() {
                if was_selected {
                    app.editor.selected.remove(&note);
                } else {
                    app.editor.selected.insert(note);
                }
                ui.close();
            }
        }
        #[cfg(target_os = "android")]
        if ui
            .add_enabled(selected, Button::new(tr!("编辑歌词")))
            .clicked()
        {
            app.edit_lyric();
            ui.close();
        }
        if ui
            .add_enabled(
                selected,
                Button::new(tr!("填入歌词…")).shortcut_text("Ctrl+L"),
            )
            .clicked()
        {
            crate::ui::fill_lyrics::open(app);
            ui.close();
        }
        if ui
            .add_enabled(selected, Button::new(tr!("静音音符")))
            .clicked()
        {
            app.editor.mute_selected();
            ui.close();
        }
        ui.separator();
        crate::ui::retakes::commands(app, ui);
        ui.separator();
        if ui
            .add_enabled(selected, Button::new(tr!("复制")).shortcut_text("Ctrl+C"))
            .clicked()
        {
            crate::ui::chrome::edit::copy(app, ui.ctx());
            ui.close();
        }
        if ui
            .add_enabled(selected, Button::new(tr!("剪切")).shortcut_text("Ctrl+X"))
            .clicked()
        {
            crate::ui::chrome::edit::cut(app, ui.ctx());
            ui.close();
        }
        if ui
            .add_enabled(
                crate::ui::chrome::edit::can_paste(app),
                Button::new(tr!("粘贴")).shortcut_text("Ctrl+V"),
            )
            .on_hover_text(tr!("在播放游标处粘贴音符"))
            .clicked()
        {
            crate::ui::chrome::edit::paste(app, ui.ctx());
            ui.close();
        }
        if ui
            .add_enabled(selected, Button::new(tr!("删除所选音符")))
            .clicked()
        {
            app.editor.delete_selected();
            ui.close();
        }
    });
    if response.drag_started_by(PointerButton::Primary) {
        start_gesture(app, ui, canvas, groups);
    }
    if response.dragged_by(PointerButton::Primary) {
        update_gesture(app, ui, canvas);
    }
    if response.drag_stopped_by(PointerButton::Primary) {
        app.editor.finish();
        app.gesture = None;
    }
}
