use super::{Viewport, grid_step, x_at};
use crate::{
    model::{TimeAxis, beats},
    ui::theme::{edge_color, text_color},
};
use eframe::egui::*;

const BEAT_LINE: Color32 = Color32::from_rgb(70, 68, 73);
const SUBDIVISION_LINE: Color32 = Color32::from_rgb(53, 52, 55);
const BAR_LINE: Color32 = Color32::from_rgb(105, 102, 108);

fn visible_end(rect: Rect, view: &Viewport) -> f64 {
    view.start + rect.width() as f64 / view.beat_width as f64
}

fn visible_steps(rect: Rect, view: &Viewport, step: f64) -> impl Iterator<Item = f64> {
    let step = grid_step(view, step);
    let start = (view.start / step).floor() as i64;
    let count = ((visible_end(rect, view) - view.start) / step).ceil() as i64 + 2;
    (start..start + count).map(move |i| i as f64 * step)
}

pub fn grid(ui: &Ui, rect: Rect, view: &Viewport, time: &TimeAxis, step: f64) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    for beat in visible_steps(rect, view, step) {
        let whole = (beat - beat.round()).abs() < 1e-5;
        painter.vline(
            x_at(rect, view, beat),
            rect.y_range(),
            Stroke::new(
                if whole { 1.0 } else { 0.75 },
                if whole { BEAT_LINE } else { SUBDIVISION_LINE },
            ),
        );
    }
    time.for_each_bar_in(
        view.start.max(0.0),
        visible_end(rect, view) + 1.0,
        |beat, _, _, _| {
            painter.vline(
                x_at(rect, view, beat),
                rect.y_range(),
                Stroke::new(1.0, BAR_LINE),
            );
        },
    );
}

pub fn draw_ruler(ui: &Ui, rect: Rect, view: &Viewport, time: &TimeAxis, step: f64) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let clip = painter.clip_rect();
    // Anchor the ticks to the baseline, independently of the surrounding header height.
    let baseline = rect.bottom();
    let beat_tick = (rect.height() * 0.5).min(16.0);
    let number_offset = (rect.height() - 9.0).clamp(9.0, 25.0);
    for beat in visible_steps(rect, view, step) {
        painter.vline(
            x_at(rect, view, beat),
            baseline - beat_tick..=baseline,
            Stroke::new(1.0, BEAT_LINE),
        );
    }
    time.for_each_bar_in(
        view.start.max(0.0),
        visible_end(rect, view) + 1.0,
        |beat, index, _, _| {
            let x = x_at(rect, view, beat);
            painter.vline(
                x,
                (baseline - 32.0).max(rect.top())..=baseline,
                Stroke::new(1.0, BAR_LINE),
            );
            if x < clip.left() - 96.0 || x > clip.right() {
                return;
            }
            let text = painter.layout_no_wrap(
                (i64::from(index) + 1).to_string(),
                FontId::proportional(12.0),
                text_color(),
            );
            let pos = pos2(x + 7.0, baseline - number_offset - text.size().y / 2.0);
            if Rect::from_min_size(pos, text.size()).intersects(clip) {
                painter.galley(pos, text, text_color());
            }
        },
    );
    painter.hline(rect.x_range(), baseline, Stroke::new(1.0, edge_color()));
}

pub fn playhead_line(ui: &Ui, rect: Rect, view: &Viewport, position: i64) {
    let x = x_at(rect, view, beats(position));
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .vline(x, rect.y_range(), Stroke::new(1.0, text_color()));
}

pub fn playhead(ui: &Ui, rect: Rect, baseline: f32, view: &Viewport, position: i64) {
    playhead_line(
        ui,
        Rect::from_min_max(pos2(rect.left(), baseline), rect.max),
        view,
        position,
    );
    let x = x_at(rect, view, beats(position));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let top = baseline - 14.0;
    painter.rect_filled(
        Rect::from_min_size(pos2(x - 4.0, top), vec2(8.0, 9.0)),
        CornerRadius {
            nw: 1,
            ne: 1,
            sw: 0,
            se: 0,
        },
        text_color(),
    );
    painter.add(Shape::convex_polygon(
        vec![
            pos2(x - 4.0, top + 8.0),
            pos2(x + 4.0, top + 8.0),
            pos2(x + 4.0, top + 9.0),
            pos2(x, baseline),
            pos2(x - 4.0, top + 9.0),
        ],
        text_color(),
        Stroke::NONE,
    ));
}
