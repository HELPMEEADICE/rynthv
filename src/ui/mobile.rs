use super::{
    child,
    icons::{Icon, IconButton},
    state::Column,
    theme, workspace,
};
use crate::app::App;
use eframe::egui::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Editor,
    Panel(Column),
    Mixer,
}

const PAGES: [(Page, &str); 6] = [
    (Page::Editor, "卷帘"),
    (Page::Panel(Column::Note), "音符"),
    (Page::Panel(Column::Voice), "歌声"),
    (Page::Panel(Column::Dictionary), "词典"),
    (Page::Mixer, "混音"),
    (Page::Panel(Column::Render), "渲染"),
];

pub(crate) const TRANSPORT_HEIGHT: f32 = 48.0;
pub(crate) const MENU_HEIGHT: f32 = 42.0;
const TABS_HEIGHT: f32 = 41.0;

fn current_page(ctx: &Context) -> Page {
    ctx.data(|data| data.get_temp::<Page>(Id::new("mobile-page")))
        .unwrap_or_default()
}

fn set_page(ctx: &Context, page: Page) {
    ctx.data_mut(|data| data.insert_temp(Id::new("mobile-page"), page));
}

pub(crate) fn back_to_editor(ctx: &Context) -> bool {
    if current_page(ctx) == Page::Editor {
        return false;
    }
    set_page(ctx, Page::Editor);
    true
}

pub(crate) fn open_render(app: &mut App, ctx: &Context) {
    if ctx.content_rect().width() < 980.0 {
        set_page(ctx, Page::Panel(Column::Render));
    } else if !app.view.columns.contains(&Column::Render) {
        app.view.toggle(Column::Render);
    }
}

pub(crate) fn workspace(app: &mut App, ui: &mut Ui) {
    let rect = ui.max_rect();
    let tabs = Rect::from_min_max(pos2(rect.left(), rect.bottom() - TABS_HEIGHT), rect.max);
    let mut page = current_page(ui.ctx());
    ui.painter().rect_filled(tabs, 4, theme::PANEL);
    child(ui, tabs, "mobile-tabs").horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let width = ui.available_width() / PAGES.len() as f32;
        for (target, label) in PAGES {
            if ui
                .add_sized(
                    vec2(width, TABS_HEIGHT - 2.0),
                    Button::new(label).selected(page == target),
                )
                .clicked()
            {
                page = target;
                set_page(ui.ctx(), target);
            }
        }
    });
    let mut content = child(
        ui,
        Rect::from_min_max(rect.min, pos2(rect.right(), tabs.top() - 3.0)),
        "mobile-workspace",
    );
    match page {
        Page::Editor => workspace::editor(app, &mut content),
        Page::Panel(column) => super::panels::show(app, &mut content, column),
        Page::Mixer => mixer(app, &mut content),
    }
}

fn mixer(app: &mut App, ui: &mut Ui) {
    ui.painter().rect_filled(ui.max_rect(), 4, theme::PANEL);
    ScrollArea::vertical().show(ui, |ui| {
        ui.set_width(ui.available_width());
        for index in 0..app.editor.project.tracks.len() {
            let width = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(vec2(width, 60.0), Sense::hover());
            super::track_header::show(app, ui, rect, index);
            ui.separator();
        }
    });
}

pub(crate) fn transport(app: &mut App, ui: &mut Ui) {
    ui.painter().rect_filled(ui.max_rect(), 4, theme::PANEL);
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let button = |icon, label| {
            IconButton::new(icon, label)
                .size(Vec2::splat(42.0))
                .icon_size(22.0)
        };
        let playing = app.editor.transport.playing;
        if ui
            .add_enabled(
                app.synthesis.recording_error.is_none(),
                button(
                    if playing { Icon::Pause } else { Icon::Play },
                    if playing { "暂停" } else { "播放" },
                ),
            )
            .clicked()
        {
            app.editor.transport.toggle();
        }
        if ui.add(button(Icon::Stop, "停止")).clicked() {
            app.editor.transport.stop();
        }
        if ui
            .add(button(Icon::Loop, "循环").selected(app.editor.transport.looping))
            .clicked()
        {
            app.editor.transport.looping = !app.editor.transport.looping;
        }
        for (label, enabled, undo) in [
            ("撤销", app.editor.can_undo(), true),
            ("重做", app.editor.can_redo(), false),
        ] {
            if ui
                .add_enabled(
                    enabled && !app.editor.is_editing(),
                    Button::new(label).min_size(vec2(42.0, 42.0)),
                )
                .clicked()
            {
                app.commit_lyric();
                if undo {
                    app.editor.undo();
                } else {
                    app.editor.redo();
                }
            }
        }
        let seconds = app
            .editor
            .project
            .time
            .seconds_at(app.editor.transport.position)
            .max(0.0);
        ui.label(format!(
            "{:02}:{:05.2}",
            (seconds / 60.0) as u64,
            seconds % 60.0
        ));
        if ui.available_width() >= 140.0 {
            ui.label(app.synthesis.status());
        }
    });
}
