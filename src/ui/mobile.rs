mod toolbar;

use super::{child, state::Column, theme, workspace};
use crate::app::App;
use eframe::egui::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Piano,
    Arrangement,
    Panel(Column),
    Mixer,
}

impl Page {
    const EDITORS: [Self; 2] = [Self::Piano, Self::Arrangement];
    const PANELS: [Self; 5] = [
        Self::Panel(Column::Note),
        Self::Panel(Column::Voice),
        Self::Panel(Column::Dictionary),
        Self::Mixer,
        Self::Panel(Column::Render),
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Piano => "卷帘",
            Self::Arrangement => "编排",
            Self::Panel(column) => column.label(),
            Self::Mixer => "混音",
        }
    }
}

fn current_page(ctx: &Context) -> Page {
    ctx.data(|data| data.get_temp::<Page>(Id::new("mobile-page")))
        .unwrap_or_default()
}

fn set_page(app: &mut App, ctx: &Context, page: Page) {
    if current_page(ctx) != page {
        app.commit_lyric();
        super::lanes::parameter_deactivate(app);
        super::phonemes::phoneme_deactivate(app);
        ctx.data_mut(|data| data.insert_temp(Id::new("mobile-page"), page));
        ctx.request_repaint();
    }
}

pub(crate) fn back_to_editor(app: &mut App, ctx: &Context) -> bool {
    if current_page(ctx) == Page::Piano {
        return false;
    }
    set_page(app, ctx, Page::Piano);
    true
}

pub(super) fn open_panel(app: &mut App, ctx: &Context, column: Column) {
    set_page(app, ctx, Page::Panel(column));
}

pub(super) use toolbar::transport_options;

pub(crate) fn show(app: &mut App, ui: &mut Ui, safe: Rect) {
    if safe.width() < 48.0 || safe.height() < 38.0 {
        return;
    }
    let rect = safe.shrink2(vec2(4.0, 0.0));
    let header_height = if rect.height() < 130.0 { 32.0 } else { 38.0 };
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), header_height));
    if app.lyric.is_none() || rect.height() >= 130.0 {
        toolbar::show(app, &mut child(ui, header, "mobile-toolbar"));
    }
    let status = app.preferences.show_status_bar
        && (rect.height() >= 520.0 || (rect.height() >= 220.0 && !app.status.is_empty()));
    let bottom = rect.bottom() - if status { 20.0 } else { 4.0 };
    let top = if app.lyric.is_some() && rect.height() < 130.0 {
        rect.top()
    } else {
        header.bottom() + 4.0
    };
    let body = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), bottom));
    if body.is_positive() {
        content(app, &mut child(ui, body, "mobile-workspace"));
    }
    if status {
        let rect = Rect::from_min_max(pos2(rect.left(), bottom), rect.max);
        let message = if app.status.is_empty() {
            app.synthesis.status()
        } else {
            &app.status
        };
        child(ui, rect, "mobile-status").add(Label::new(RichText::new(message).weak()).truncate());
    }
}

fn content(app: &mut App, ui: &mut Ui) {
    match current_page(ui.ctx()) {
        Page::Piano => {
            let rect = ui.max_rect();
            if rect.height() >= 480.0 && !app.view.arrangement_collapsed && app.lyric.is_none() {
                let arrangement_height = (rect.height() * 0.32).clamp(154.0, 260.0);
                let (piano, arrangement) = if app.view.arrangement_first {
                    let y = rect.top() + arrangement_height;
                    (
                        Rect::from_min_max(pos2(rect.left(), y + 4.0), rect.max),
                        Rect::from_min_max(rect.min, pos2(rect.right(), y)),
                    )
                } else {
                    let y = rect.bottom() - arrangement_height;
                    (
                        Rect::from_min_max(rect.min, pos2(rect.right(), y - 4.0)),
                        Rect::from_min_max(pos2(rect.left(), y), rect.max),
                    )
                };
                super::arrangement::show(app, &mut child(ui, arrangement, "mobile-arrangement"));
                workspace::piano_stack(app, &mut child(ui, piano, "mobile-piano"));
            } else {
                workspace::piano_stack(app, ui);
            }
        }
        Page::Arrangement => super::arrangement::show(app, ui),
        Page::Panel(column) => super::panels::show(app, ui, column),
        Page::Mixer => mixer(app, ui),
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
