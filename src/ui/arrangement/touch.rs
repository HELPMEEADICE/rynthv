use crate::app::App;
use eframe::egui::*;

pub(super) fn navigate(app: &mut App, ui: &Ui, timeline: Rect, left: f32) -> (bool, f32) {
    let ctx = ui.ctx();
    let id = Id::new("arrangement-touch-navigation");
    let captured = ctx.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    if !ctx.input(|input| input.any_touches()) {
        if captured {
            ctx.data_mut(|data| data.remove::<bool>(id));
            if ctx.dragged_id() == Some(id) {
                ctx.stop_dragging();
            }
        }
        return (captured, 0.0);
    }
    let Some(touch) = ctx.multi_touch() else {
        return (captured, 0.0);
    };
    if !captured {
        let body = Rect::from_min_max(pos2(left, timeline.top()), timeline.max);
        if !body.intersect(ui.clip_rect()).contains(touch.start_pos) || Popup::is_any_open(ctx) {
            return (false, 0.0);
        }
        ctx.data_mut(|data| data.insert_temp(id, true));
        if app
            .group_gesture
            .as_ref()
            .is_some_and(|gesture| gesture.arrangement)
            || app.audio_drag.is_some()
        {
            app.editor.cancel();
            app.group_gesture = None;
            app.audio_drag = None;
        }
        app.view_pan.cancel(ctx);
    }
    ctx.set_dragged_id(id);
    let previous_x = touch.center_pos.x - touch.translation_delta.x - timeline.left();
    let current_x = touch.center_pos.x - timeline.left();
    let view = &mut app.view.arrangement;
    let beat = view.start + f64::from(previous_x / view.beat_width);
    view.beat_width = (view.beat_width * touch.zoom_delta_2d.x).clamp(8.0, 640.0);
    view.start = (beat - f64::from(current_x / view.beat_width)).max(0.0);
    (true, touch.translation_delta.y)
}
