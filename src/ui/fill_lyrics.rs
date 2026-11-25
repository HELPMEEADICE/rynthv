use crate::{
    app::App,
    editor::{LyricSelection, split_lyrics},
    ui::theme::*,
};
use eframe::egui::{
    text::{CCursor, CCursorRange},
    *,
};

pub struct FillLyrics {
    selection: LyricSelection,
    text: String,
    repeat: bool,
    by_character: bool,
    focus: bool,
}

pub fn open(app: &mut App) {
    super::tempo_input::finish(app);
    app.commit_lyric();
    app.editor.finish();
    if let Some((selection, text)) = app.editor.lyric_selection() {
        app.fill_lyrics = Some(FillLyrics {
            selection,
            text,
            repeat: false,
            by_character: true,
            focus: true,
        });
    }
}

pub fn show(app: &mut App, ctx: &Context) {
    let Some(mut edit) = app.fill_lyrics.take() else {
        return;
    };
    #[cfg(target_os = "android")]
    let (width, input_height) = (
        super::dialog::width(ctx, 400.0),
        (ctx.content_rect().height() * 0.28).clamp(32.0, 180.0),
    );
    #[cfg(not(target_os = "android"))]
    let (width, input_height) = (
        (ctx.content_rect().width() - 80.0).clamp(200.0, 400.0),
        (ctx.content_rect().height() - 320.0).clamp(80.0, 260.0),
    );
    let mut submit = false;
    let mut cancel = false;
    let mut words = Vec::new();
    let modal = Modal::new(Id::new("fill-lyrics"))
        .frame(
            Frame::new()
                .fill(Color32::from_rgb(55, 55, 57))
                .stroke(Stroke::new(1.0, Color32::from_rgb(73, 72, 76)))
                .corner_radius(4)
                .inner_margin(super::dialog::MARGIN),
        )
        .backdrop_color(Color32::from_black_alpha(30))
        .show(ctx, |ui| {
            super::dialog::body(ui, width, |ui| {
                ui.spacing_mut().item_spacing.y = 12.0;
                ui.label(
                    RichText::new("填入歌词")
                        .font(FontId::new(17.0, FontFamily::Name("SectionTitle".into()))),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new("用空格隔开字词。\n每个音符对应一个字词。").color(muted_color()),
                );
                let input_id = Id::new("fill-lyrics-text");
                if edit.focus {
                    TextEdit::store_state(ctx, input_id, Default::default());
                }
                Frame::new()
                    .fill(panel_color())
                    .stroke(Stroke::new(1.0, muted_color()))
                    .corner_radius(4)
                    .inner_margin(5)
                    .show(ui, |ui| {
                        ScrollArea::vertical()
                            .id_salt("fill-lyrics-scroll")
                            .max_height(input_height)
                            .min_scrolled_height(input_height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let input = ui.add(
                                    TextEdit::multiline(&mut edit.text)
                                        .id(input_id)
                                        .desired_width(f32::INFINITY)
                                        .min_size(vec2(0.0, input_height))
                                        .frame(Frame::NONE)
                                        .margin(0.0),
                                );
                                if edit.focus {
                                    input.request_focus();
                                    if let Some(mut state) = TextEdit::load_state(ctx, input.id) {
                                        state.cursor.set_char_range(Some(CCursorRange::two(
                                            CCursor::new(0),
                                            CCursor::new(edit.text.chars().count()),
                                        )));
                                        state.store(ctx, input.id);
                                    }
                                    edit.focus = false;
                                    ctx.request_repaint();
                                }
                            });
                    });
                ui.checkbox(&mut edit.repeat, "循环填入");
                ui.checkbox(&mut edit.by_character, "逐字填入")
                    .on_hover_text("拆分汉字、假名和韩文音节；拼音、罗马字及英文按空格分词。");
                words = split_lyrics(&edit.text, edit.by_character);
                let notes = edit.selection.len();
                let hint = if words.is_empty() {
                    format!("已选择 {notes} 个音符，请输入歌词。")
                } else if edit.repeat {
                    format!("{} 个字词 · 循环填入 {notes} 个音符", words.len())
                } else if words.len() > notes {
                    format!(
                        "填入 {notes} 个音符 · 多出的 {} 个字词不填入",
                        words.len() - notes
                    )
                } else if words.len() < notes {
                    format!("填入 {} / {notes} 个音符 · 其余保留原歌词", words.len())
                } else {
                    format!("{} 个字词 · 填入 {notes} 个音符", words.len())
                };
                ui.label(RichText::new(hint).size(11.0).color(muted_color()))
                    .on_hover_text("更改歌词时会清除对应音符的自定义音素，以便重新生成发音。");
                ui.add_space(8.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let size = vec2(((width - 8.0) / 2.0).clamp(1.0, 106.0), 34.0);
                    submit = ui
                        .add_enabled(
                            !words.is_empty(),
                            Button::new(RichText::new("确定").color(canvas_color()))
                                .fill(green_color())
                                .min_size(size),
                        )
                        .on_hover_text("Ctrl+Enter 确定")
                        .clicked();
                    cancel = ui
                        .add_sized(size, Button::new("取消").fill(button_hover_color()))
                        .clicked();
                });
            })
        });
    if modal.is_top_modal && !modal.any_popup_open {
        cancel |= ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        submit |= ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Enter));
    }
    if cancel {
        ctx.memory_mut(|m| m.surrender_focus(Id::new("fill-lyrics-text")));
        return;
    }
    if submit && !words.is_empty() {
        let changed = app.editor.fill_lyrics(&edit.selection, &words, edit.repeat);
        app.status = format!("已更新 {changed} 个音符的歌词");
        ctx.memory_mut(|m| m.surrender_focus(Id::new("fill-lyrics-text")));
    } else {
        app.fill_lyrics = Some(edit);
    }
}
