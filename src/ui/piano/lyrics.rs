use super::LyricEdit;
#[cfg(target_os = "android")]
use crate::tr;
use crate::{app::App, model::*};
use eframe::egui::{
    text::{CCursor, CCursorRange},
    *,
};
use std::collections::BTreeSet;
#[cfg(not(target_os = "android"))]
pub(super) fn lyric(app: &mut App, ui: &mut Ui, rect: Rect, reference: &GroupRef) {
    let Some(edit) = app.lyric.take() else {
        return;
    };
    let Some(note) = app.editor.notes().iter().find(|n| n.id == edit.id) else {
        return;
    };
    let Some(mut r) = crate::ui::group_geometry::visible_note_rect(
        note,
        reference,
        rect,
        &app.view.piano,
        app.view.top_pitch,
        app.view.row_height,
    ) else {
        app.lyric = Some(edit);
        return;
    };
    let clip = crate::ui::group_geometry::clip_rect(reference, rect, &app.view.piano);
    r.max.x = r.max.x.max(r.min.x + 130.0).min(clip.right());
    let mut edit_ui = crate::ui::child(ui, r, ("lyric", edit.id));
    field(app, &mut edit_ui, r.size(), reference, edit);
}

#[cfg(target_os = "android")]
pub(super) fn toolbar(app: &mut App, ui: &mut Ui) {
    let Some(edit) = app.lyric.take() else {
        return;
    };
    let Some(reference) = app.editor.active_ref().cloned() else {
        return;
    };
    ui.horizontal_centered(|ui| {
        ui.add_space(5.0);
        ui.label(tr!("歌词"));
        let size = vec2((ui.available_width() - 52.0).max(30.0), 30.0);
        field(app, ui, size, &reference, edit);
        if ui
            .add(Button::new(tr!("完成")).min_size(vec2(44.0, 30.0)))
            .clicked()
        {
            app.commit_lyric();
        }
    });
}

fn field(app: &mut App, ui: &mut Ui, size: Vec2, reference: &GroupRef, mut edit: LyricEdit) {
    let response = ui.add_sized(
        size,
        TextEdit::singleline(&mut edit.text)
            .id(Id::new(("lyric-input", edit.id)))
            .margin(vec2(4.0, 1.0))
            .vertical_align(Align::Center),
    );
    if edit.focus {
        response.request_focus();
        if let Some(mut state) = TextEdit::load_state(ui.ctx(), response.id) {
            state.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(0),
                CCursor::new(edit.text.chars().count()),
            )));
            state.store(ui.ctx(), response.id);
        }
        edit.focus = false;
        ui.ctx().request_repaint();
    }
    let escape = ui.input(|i| i.key_pressed(Key::Escape));
    let tab = ui.input(|i| i.key_pressed(Key::Tab));
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    if escape {
        response.surrender_focus();
    } else if enter || tab || response.lost_focus() {
        app.editor.set_lyric(edit.id, edit.text);
        response.surrender_focus();
        if tab {
            let mut notes: Vec<_> = app
                .editor
                .notes()
                .iter()
                .filter(|n| reference.note_range(n).is_some())
                .collect();
            notes.sort_by_key(|n| n.onset);
            if let Some(next) = notes
                .iter()
                .position(|n| n.id == edit.id)
                .and_then(|i| notes.get(i + 1))
            {
                app.lyric = Some(LyricEdit {
                    id: next.id,
                    text: next.lyrics.clone(),
                    focus: true,
                });
                app.editor.selected = BTreeSet::from([next.id]);
            }
        }
    } else {
        app.lyric = Some(edit);
    }
}
