use crate::app::App;
use eframe::egui::*;

pub(super) fn transform(app: &mut App, ui: &Ui, canvas: Rect) -> bool {
    let ctx = ui.ctx();
    let id = ui.make_persistent_id("piano-touch-transform");
    let captured = ctx
        .data(|data| data.get_temp::<bool>(id))
        .unwrap_or_default();
    let touches = ctx.input(|input| input.any_touches());
    if !touches {
        ctx.data_mut(|data| data.remove::<bool>(id));
        return captured;
    }
    let Some(touch) = ctx.multi_touch() else {
        return captured;
    };
    if !captured {
        if !canvas.intersect(ui.clip_rect()).contains(touch.start_pos) || Popup::is_any_open(ctx) {
            return false;
        }
        ctx.data_mut(|data| data.insert_temp(id, true));
        if let Some(super::Gesture::Select { initial, .. }) = app.gesture.take() {
            app.editor.selected = initial;
        } else if app.editor.is_editing() {
            app.editor.cancel();
        }
        app.gesture = None;
        app.group_gesture = None;
        crate::ui::piano::cancel_audition(app);
        crate::ui::lanes::parameter_deactivate(app);
        crate::ui::phonemes::phoneme_deactivate(app);
        app.view_pan.cancel(ctx);
    }
    let previous = touch.center_pos - touch.translation_delta - canvas.min.to_vec2();
    let center = touch.center_pos - canvas.min.to_vec2();
    let view = &mut app.view;
    let beat = view.piano.start + f64::from(previous.x / view.piano.beat_width);
    let pitch = view.top_pitch - previous.y / view.row_height;
    view.piano.beat_width = (view.piano.beat_width * touch.zoom_delta_2d.x).clamp(8.0, 640.0);
    view.row_height = (view.row_height * touch.zoom_delta_2d.y).clamp(8.0, 48.0);
    view.piano.start = (beat - f64::from(center.x / view.piano.beat_width)).max(0.0);
    view.top_pitch = (pitch + center.y / view.row_height).clamp(12.0, 127.0);
    true
}
