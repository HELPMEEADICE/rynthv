use crate::{app::App, tr, ui};
use eframe::egui::*;

pub(crate) fn show(app: &mut App, ui: &mut Ui) {
    let rect = ui.max_rect();
    let menu = Rect::from_min_size(rect.min, vec2(rect.width(), 27.0));
    super::menus(app, &mut ui::child(ui, menu, "menu"));
    let transport = Rect::from_min_size(pos2(rect.left(), menu.bottom()), vec2(rect.width(), 44.0));
    ui::transport::show(app, &mut ui::child(ui, transport, "transport"));
    let status_height = if app.preferences.show_status_bar {
        23.0
    } else {
        0.0
    };
    let status = Rect::from_min_size(
        pos2(rect.left(), rect.bottom() - status_height),
        vec2(rect.width(), status_height),
    );
    let work = Rect::from_min_max(
        pos2(rect.left() + 5.0, transport.bottom()),
        pos2(rect.right() - 5.0, status.top() - 4.0),
    );
    ui::workspace::show(app, &mut ui::child(ui, work, "workspace"));
    if app.preferences.show_status_bar {
        ui::child(ui, status.shrink2(vec2(8.0, 1.0)), "status").horizontal(|ui| {
            let hint = if app.status.is_empty() && app.piano_tool == ui::piano::Tool::PitchAnchor {
                tr!("双击创建控制点 · 拖动框选 / 移动 · 四边手柄缩放 · Delete 删除 · Esc 取消 · Ctrl+Z 撤销")
            } else if app.status.is_empty() && app.piano_tool == ui::piano::Tool::PitchDraw {
                tr!("左键拖动绘制音高 · 右键拖动擦除 · Esc 取消本次绘制 · Ctrl+Z 撤销 · 中键拖动视野")
            } else if app.status.is_empty() {
                tr!("双击创建音符 · 拖动框选 · 中键拖动视野 · Shift+滚轮水平移动 · Ctrl+滚轮缩放")
            } else {
                &app.status
            };
            ui.label(RichText::new(hint).size(11.0).color(ui::theme::muted_color()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(app.synthesis.status()).size(11.0).color(ui::theme::muted_color()));
            });
        });
    }
}
