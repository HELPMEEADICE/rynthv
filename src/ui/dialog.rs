use eframe::egui::*;

pub(super) const MARGIN: i8 = if cfg!(target_os = "android") { 10 } else { 24 };

#[cfg(target_os = "android")]
pub(super) fn width(ctx: &Context, preferred: f32) -> f32 {
    (ctx.content_rect().width() - 2.0 * f32::from(MARGIN) - 8.0).clamp(1.0, preferred)
}

pub(super) fn body(ui: &mut Ui, width: f32, contents: impl FnOnce(&mut Ui)) {
    ui.set_width(width);
    #[cfg(target_os = "android")]
    let height = (ui.ctx().content_rect().height() - 2.0 * f32::from(MARGIN) - 8.0).max(1.0);
    #[cfg(target_os = "android")]
    ScrollArea::vertical()
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, contents);
    #[cfg(not(target_os = "android"))]
    contents(ui);
}
