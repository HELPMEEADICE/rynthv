#[cfg(not(target_os = "android"))]
mod desktop;
pub(crate) mod edit;
use super::state::Column;
use crate::{
    app::{App, FileAction, MidiImportMode},
    model::*,
};
#[cfg(not(target_os = "android"))]
pub(crate) use desktop::show as desktop;
use eframe::egui::*;

fn item(ui: &mut Ui, text: &str, shortcut: &str, enabled: bool) -> bool {
    let response = ui.add_enabled(
        enabled,
        Button::new(text)
            .shortcut_text(shortcut)
            .min_size(vec2(180.0, 22.0)),
    );
    if response.clicked() {
        ui.close();
        true
    } else {
        false
    }
}
#[cfg(not(target_os = "android"))]
fn menus(app: &mut App, ui: &mut Ui) {
    MenuBar::new().ui(ui, |ui| {
        ui.spacing_mut().button_padding = vec2(11.0, 4.0);
        ui.spacing_mut().item_spacing.x = 0.0;
        file_menu(app, ui);
        edit_menu(app, ui);
        other_menus(app, ui);
    });
}

#[cfg(any(target_os = "android", target_arch = "wasm32"))]
pub(super) fn mobile_menus(app: &mut App, ui: &mut Ui, separate: bool) {
    if separate {
        file_menu(app, ui);
        edit_menu(app, ui);
    }
    ui.menu_button("菜单", |ui| {
        if !separate {
            file_menu(app, ui);
            edit_menu(app, ui);
        }
        other_menus(app, ui);
    });
}

fn file_menu(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    ui.menu_button("文件", |ui| {
        if item(ui, "新建", "Ctrl+N", true) {
            app.request_file(FileAction::New, &ctx);
        }
        if item(ui, "打开…", "Ctrl+O", true) {
            app.request_file(FileAction::Open, &ctx);
        }
        #[cfg(any(target_os = "android", target_arch = "wasm32"))]
        if item(
            ui,
            if cfg!(target_arch = "wasm32") {
                "浏览器工程…"
            } else {
                "本机工程…"
            },
            "",
            true,
        ) {
            app.request_file(FileAction::OpenLocal, &ctx);
        }
        ui.separator();
        if item(ui, "保存", "Ctrl+S", true) {
            app.save_project(false, &ctx);
        }
        if item(
            ui,
            if cfg!(target_arch = "wasm32") {
                "下载 SVP…"
            } else {
                "另存为…"
            },
            "Ctrl+Shift+S",
            true,
        ) {
            app.save_project(true, &ctx);
        }
        #[cfg(target_os = "android")]
        if item(ui, "分享工程…", "", true) {
            app.share_project(&ctx);
        }
        ui.separator();
        if item(ui, "导入…", "Ctrl+Shift+I", true) {
            app.request_file(FileAction::ImportMidi, &ctx);
        }
        if item(ui, "作为音轨导入…", "", true) {
            app.commit_lyric();
            app.begin_midi_import(MidiImportMode::Tracks, &ctx);
        }
        if item(ui, "导入音频…", "", true) {
            app.commit_lyric();
            app.import_audio(None, &ctx);
        }
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        item(ui, "导出音频", "", false);
        #[cfg(target_os = "android")]
        if item(ui, "导出音频…", "", true) {
            super::mobile::open_panel(app, &ctx, Column::Render);
        }
        #[cfg(target_arch = "wasm32")]
        if item(ui, "导出音频…", "", true) {
            if super::mobile::compact(&ctx) {
                super::mobile::open_panel(app, &ctx, Column::Render);
            } else if !app.view.columns.contains(&Column::Render) {
                app.view.toggle(Column::Render);
            }
        }
    });
}

fn edit_menu(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    ui.menu_button("编辑", |ui| {
        let idle = !app.editor.is_editing();
        if item(
            ui,
            &format!("撤销 {}", app.editor.undo_label()),
            "Ctrl+Z",
            idle && app.editor.can_undo(),
        ) {
            app.commit_lyric();
            app.editor.undo();
        }
        if item(ui, "重做", "Ctrl+Y", idle && app.editor.can_redo()) {
            app.commit_lyric();
            app.editor.redo();
        }
        ui.separator();
        if item(ui, "全选", "Ctrl+A", idle) {
            app.commit_lyric();
            edit::select_all(app);
        }
        if item(ui, "取消选择", "Esc", idle) {
            app.commit_lyric();
            edit::clear(app);
        }
        let selected = idle && edit::selected(app);
        if item(ui, "删除所选内容", "Delete", selected) {
            app.commit_lyric();
            edit::delete(app);
        }
        ui.separator();
        if item(ui, "复制", "Ctrl+C", selected) {
            app.commit_lyric();
            edit::copy(app, &ctx);
        }
        if item(ui, "剪切", "Ctrl+X", selected) {
            app.commit_lyric();
            edit::cut(app, &ctx);
        }
        if item(ui, "粘贴", "Ctrl+V", idle && edit::can_paste(app)) {
            app.commit_lyric();
            edit::paste(app, &ctx);
        }
    });
}

fn other_menus(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    ui.menu_button("查看", |ui| {
        ui.checkbox(&mut app.show_time_tracks, "显示曲速和拍号轨");
        #[cfg(not(target_os = "android"))]
        if ui
            .checkbox(&mut app.view.arrangement_collapsed, "折叠编排")
            .clicked()
        {
            ui.close();
        }
        #[cfg(target_os = "android")]
        {
            let mut show = !app.view.arrangement_collapsed;
            if ui.checkbox(&mut show, "空间充足时同时显示编排").changed() {
                app.view.arrangement_collapsed = !show;
            }
        }
        ui.checkbox(&mut app.view.arrangement_first, "编排位于上方");
        ui.separator();
        for column in Column::PANELS {
            #[cfg(not(target_os = "android"))]
            {
                let mut active = app.view.columns.contains(&column);
                if ui.checkbox(&mut active, column.label()).clicked() {
                    app.view.toggle(column);
                }
            }
            #[cfg(target_os = "android")]
            if item(ui, column.label(), "", true) {
                super::mobile::open_panel(app, &ctx, column);
            }
        }
        ui.separator();
        if item(ui, "设置", "", true) {
            app.settings = true;
        }
    });
    ui.menu_button("修改", |ui| {
        let selected = !app.editor.is_editing()
            && !super::lanes::parameter_active(app)
            && !super::phonemes::phoneme_active(app)
            && app.piano_tool != super::piano::Tool::PitchAnchor
            && !app.editor.selected.is_empty();
        if item(ui, "填入歌词…", "Ctrl+L", selected) {
            app.commit_lyric();
            super::fill_lyrics::open(app);
        }
        if item(ui, "静音音符", "Ctrl+M", selected) {
            app.commit_lyric();
            app.editor.mute_selected();
        }
        ui.separator();
        if item(
            ui,
            "升高八度",
            "Ctrl+↑",
            !app.editor.is_editing() && edit::can_transpose(app),
        ) {
            app.commit_lyric();
            edit::transpose(app, true);
        }
        if item(
            ui,
            "降低八度",
            "Ctrl+↓",
            !app.editor.is_editing() && edit::can_transpose(app),
        ) {
            app.commit_lyric();
            edit::transpose(app, false);
        }
        item(ui, "智能量化", "", false);
        item(ui, "解除组链接", "", false);
    });
    ui.menu_button("自动处理", |ui| {
        item(ui, "装饰所选音符", "", false);
        ui.add_enabled_ui(
            !app.editor.is_editing()
                && !super::lanes::parameter_active(app)
                && !super::phonemes::phoneme_active(app),
            |ui| {
                super::retakes::commands(app, ui);
            },
        );
        item(ui, "从音频提取音符", "", false);
        item(ui, "检测曲速", "", false);
    });
    ui.menu_button("项目", |ui| {
        let idle = !app.editor.is_editing();
        if item(ui, "创建音轨", "", idle) {
            app.commit_lyric();
            app.editor.add_track();
        }
        if item(
            ui,
            "在走带处插入新组",
            "",
            idle && !app.editor.current_track().is_instrumental(),
        ) {
            app.commit_lyric();
            app.editor.add_group();
        }
        if item(ui, "创建伴奏轨", "", idle) {
            app.commit_lyric();
            let id = app.editor.add_instrumental_track();
            app.import_audio(Some(id), &ctx);
        }
        item(ui, "设置歌声", "", false);
    });
    ui.menu_button("走带控制", |ui| {
        if item(
            ui,
            "播放 / 暂停",
            "Space",
            app.synthesis.recording_error.is_none(),
        ) {
            app.editor.transport.toggle();
        }
        if item(ui, "停止", "Ctrl+Space", true) {
            app.editor.transport.stop();
        }
        ui.checkbox(&mut app.editor.transport.looping, "循环播放");
        if item(ui, "跳到开头", "", true) {
            app.editor.transport.position = 0;
        }
        if item(ui, "跳到结尾", "Ctrl+End", true) {
            app.editor.transport.position = app.editor.project.end();
        }
        #[cfg(target_os = "android")]
        super::mobile::transport_options(app, ui);
    });
    ui.menu_button("脚本", |ui| {
        item(ui, "重新扫描", "", false);
        item(ui, "打开脚本文件夹", "", false);
    });
    ui.menu_button("其他", |ui| {
        if item(ui, "关于 RynthV", "", true) {
            app.about = true;
        }
    });
}
pub fn shortcuts(app: &mut App, ctx: &Context) {
    if super::phonemes::phoneme_dragging(app) {
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            super::phonemes::phoneme_cancel(app, ctx);
        }
        return;
    }
    if app.editor.has_performance_edit() || app.editor.has_voice_edit() {
        return;
    }
    if app.audio_drag.is_some() {
        return;
    }
    if app.view_pan.is_active() {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            app.view_pan.cancel(ctx);
        }
        return;
    }
    if !ctx.text_edit_focused()
        && app.lyric.is_none()
        && app.gesture.is_none()
        && app.group_gesture.is_none()
        && edit::clipboard_events(app, ctx)
    {
        return;
    }
    if ctx.egui_wants_keyboard_input() || app.lyric.is_some() {
        return;
    }
    let command = |key, modifiers| ctx.input_mut(|i| i.consume_key(modifiers, key));
    let ctrl = Modifiers::CTRL;
    let shift = Modifiers::CTRL | Modifiers::SHIFT;
    if matches!(app.gesture, Some(super::piano::Gesture::Anchor(_)))
        && super::piano::anchor_keyboard(app, ctx)
    {
        return;
    }
    if app.gesture.is_some() || app.group_gesture.is_some() {
        if command(Key::Escape, Modifiers::NONE) {
            if matches!(app.gesture, Some(super::piano::Gesture::Parameter(_))) {
                super::lanes::parameter_cancel(app, ctx);
                return;
            }
            app.editor.cancel();
            app.gesture = None;
            app.group_gesture = None;
        }
        return;
    }
    if command(Key::S, shift) {
        app.save_project(true, ctx);
    } else if command(Key::S, ctrl) {
        app.save_project(false, ctx);
    }
    if command(Key::I, shift) {
        app.request_file(FileAction::ImportMidi, ctx);
        return;
    }
    if command(Key::N, ctrl) {
        app.request_file(FileAction::New, ctx);
    }
    if command(Key::O, ctrl) {
        app.request_file(FileAction::Open, ctx);
    }
    if app.editor.is_editing() {
        return;
    }
    if command(Key::Z, shift) || command(Key::Y, ctrl) {
        app.editor.redo();
    } else if command(Key::Z, ctrl) {
        app.editor.undo();
    }
    if super::lanes::parameter_keyboard(app, ctx) {
        return;
    }
    if !super::lanes::parameter_active(app)
        && !super::phonemes::phoneme_active(app)
        && super::piano::anchor_keyboard(app, ctx)
    {
        return;
    }
    if !super::lanes::parameter_active(app)
        && !super::phonemes::phoneme_active(app)
        && command(Key::T, Modifiers::ALT | Modifiers::SHIFT)
    {
        super::retakes::prune(app);
        return;
    } else if !super::lanes::parameter_active(app)
        && !super::phonemes::phoneme_active(app)
        && command(Key::T, Modifiers::ALT)
    {
        super::retakes::create(app, RetakeComponent::All);
        return;
    }
    if command(Key::A, ctrl) {
        edit::select_all(app);
    }
    if command(Key::D, ctrl) {
        edit::duplicate(app);
    }
    if !super::lanes::parameter_active(app)
        && !super::phonemes::phoneme_active(app)
        && app.piano_tool != super::piano::Tool::PitchAnchor
        && command(Key::L, ctrl)
    {
        super::fill_lyrics::open(app);
        return;
    }
    if !super::lanes::parameter_active(app)
        && !super::phonemes::phoneme_active(app)
        && app.piano_tool != super::piano::Tool::PitchAnchor
        && command(Key::M, ctrl)
    {
        app.editor.mute_selected();
    }
    if command(Key::B, ctrl) {
        #[cfg(not(target_os = "android"))]
        app.view.toggle(Column::Note);
        #[cfg(target_os = "android")]
        super::mobile::open_panel(app, ctx, Column::Note);
    }
    if command(Key::ArrowUp, ctrl) {
        edit::transpose(app, true);
    }
    if command(Key::ArrowDown, ctrl) {
        edit::transpose(app, false);
    }
    if command(Key::Delete, Modifiers::NONE) {
        edit::delete(app);
    }
    if command(Key::Escape, Modifiers::NONE) {
        edit::clear(app);
    }
    if command(Key::Num1, Modifiers::ALT) {
        app.piano_tool = super::piano::Tool::NoteSelect;
        app.status.clear();
    }
    if command(Key::Num2, Modifiers::ALT) {
        app.piano_tool = super::piano::Tool::NoteDraw;
        app.status.clear();
    }
    if command(Key::Num3, Modifiers::ALT) {
        app.commit_lyric();
        app.editor.selected.clear();
        app.piano_tool = super::piano::Tool::PitchAnchor;
        app.show_pitch = true;
        app.status.clear();
    }
    if command(Key::Num4, Modifiers::ALT) {
        app.commit_lyric();
        app.piano_tool = super::piano::Tool::PitchDraw;
        app.show_pitch = true;
        app.status.clear();
    }
    if command(Key::Space, ctrl) {
        app.editor.transport.stop();
    } else if command(Key::Space, Modifiers::NONE) && app.synthesis.recording_error.is_none() {
        app.editor.transport.toggle();
    }
    if command(Key::End, ctrl) {
        app.editor.transport.position = app.editor.project.end();
    }
    for (key, direction) in [(Key::ArrowLeft, -1), (Key::ArrowRight, 1)] {
        if command(key, Modifiers::NONE) {
            app.editor.transport.position =
                (app.editor.transport.position + direction * ticks(app.view.step())).max(0);
        }
    }
}
