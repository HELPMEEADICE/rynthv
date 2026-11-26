mod controls;
mod dictionary;
mod note;
mod render;
pub(crate) use note::{PhonemeDraft, end_performance_frame};

use super::{child, state::Column, theme::*};
use crate::{app::App, tr};
use eframe::egui::*;

pub fn show(app: &mut App, ui: &mut Ui, column: Column) {
    let rect = ui.max_rect();
    if ui.rect_contains_pointer(rect) && ui.input(|input| input.pointer.any_pressed()) {
        super::lanes::parameter_deactivate(app);
        super::phonemes::phoneme_deactivate(app);
    }
    ui.painter().rect_filled(rect, 4, panel_color());
    let header_height = if cfg!(target_os = "android") {
        34.0
    } else {
        42.0
    };
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), header_height));
    let mut header_ui = child(
        ui,
        header.shrink2(vec2(14.0, 2.0)),
        ("panel-header", column),
    );
    header_ui.spacing_mut().interact_size.y = header.height() - 4.0;
    header_ui.horizontal(|ui| {
        let (icon, _) = ui.allocate_exact_size(vec2(20.0, 24.0), Sense::hover());
        column.icon().paint(
            ui,
            Rect::from_center_size(icon.center(), Vec2::splat(20.0)),
            muted_color(),
        );
        #[cfg(not(target_os = "android"))]
        super::section_title(
            ui,
            column.label(),
            super::state::DragPayload::Column(column),
        );
        #[cfg(target_os = "android")]
        ui.label(RichText::new(column.label()).font(section_title_font()));
    });
    ui.painter().hline(
        header.x_range(),
        header.bottom(),
        Stroke::new(1.0, edge_color()),
    );
    let mut content = child(
        ui,
        Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max),
        ("panel-content", column),
    );
    ScrollArea::vertical()
        .id_salt(("panel-scroll", column))
        .show(&mut content, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.spacing_mut().interact_size.y = 28.0;
            ui.visuals_mut().widgets.inactive.corner_radius = CornerRadius::same(4);
            if app.editor.current_track().is_instrumental()
                && matches!(column, Column::Note | Column::Voice | Column::Dictionary)
            {
                controls::padded(ui, |ui| {
                    ui.weak(tr!("当前选择的是伴奏轨，请选择歌声音轨以编辑此项。"));
                });
                return;
            }
            match column {
                Column::Note => note::show(app, ui),
                Column::Voice => super::voice_library::show(app, ui),
                Column::Render => render::show(app, ui),
                Column::Dictionary => dictionary::show(app, ui, rect.height() - header.height()),
                Column::Script => controls::padded(ui, |ui| {
                    ui.weak(tr!("脚本运行环境尚未接入"));
                }),
                Column::Editor => {}
            }
        });
    ui.painter().rect_stroke(
        rect,
        4,
        Stroke::new(1.0, border_color()),
        StrokeKind::Inside,
    );
}
