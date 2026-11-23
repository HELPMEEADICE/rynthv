use super::{geometry, *};
use crate::ui::{group_geometry::clip_rect, theme::GREEN};

pub(in crate::ui::piano) fn draw(app: &App, ui: &Ui, canvas: Rect) {
    if app.piano_tool != Tool::PitchAnchor {
        return;
    }
    let Some(reference) = app.editor.active_ref() else {
        return;
    };
    let Some(group) = app.editor.group() else {
        return;
    };
    let clip = clip_rect(reference, canvas, &app.view.piano).intersect(ui.clip_rect());
    if !clip.is_positive() {
        return;
    }
    let painter = ui.painter().with_clip_rect(clip);
    if let Some(rect) = geometry::selected_rect(app, canvas, reference) {
        if rect.width() > 14.01 || rect.height() > 14.01 {
            painter.rect_filled(rect, 3, Color32::from_white_alpha(24));
            painter.rect_stroke(
                rect,
                3,
                Stroke::new(1.0, Color32::from_white_alpha(105)),
                StrokeKind::Inside,
            );
        }
        for (_, handle) in geometry::handles(rect) {
            painter.rect_filled(handle, 1, Color32::WHITE);
        }
    }
    let draw_point = |point: &PitchPoint| {
        let position = geometry::screen(app, canvas, reference, point.pos, point.pitch);
        if !clip.expand(6.0).contains(position) {
            return;
        }
        let selected = app.piano_anchor.ids.contains(&point.id);
        let color = if selected { Color32::WHITE } else { GREEN };
        painter.circle_filled(position, 3.0, color);
        painter.circle_stroke(
            position,
            3.0,
            Stroke::new(0.7, Color32::from_black_alpha(100)),
        );
    };
    let vis_left = crate::model::ticks(crate::ui::timeline::beat_at(
        canvas,
        &app.view.piano,
        clip.left(),
    )) - reference.blick_offset;
    let vis_right = crate::model::ticks(crate::ui::timeline::beat_at(
        canvas,
        &app.view.piano,
        clip.right(),
    )) - reference.blick_offset;
    for control in &group.pitch_controls {
        match control {
            PitchControl::Point(point) => draw_point(point),
            PitchControl::Curve(curve) if app.piano_anchor.ids.contains(&curve.id) => {
                let Some((start, end)) = curve.range() else {
                    continue;
                };
                let start = start.max(vis_left);
                let end = end.min(vis_right);
                if start >= end {
                    continue;
                }
                let mut points = Vec::new();
                if let Some(pitch) = curve.sample_tick(start) {
                    points.push(geometry::screen(app, canvas, reference, start, pitch));
                }
                let local_start = start.saturating_sub(curve.pos);
                let local_end = end.saturating_sub(curve.pos);
                let first = curve.points.partition_point(|p| p.0 <= local_start);
                let stop = curve.points.partition_point(|p| p.0 < local_end);
                points.extend(
                    curve.points[first..stop]
                        .iter()
                        .filter_map(|(offset, delta)| {
                            curve.pos.checked_add(*offset).map(|time| {
                                geometry::screen(app, canvas, reference, time, curve.pitch + delta)
                            })
                        }),
                );
                if let Some(pitch) = curve.sample_tick(end) {
                    points.push(geometry::screen(app, canvas, reference, end, pitch));
                }
                if points.len() >= 2 {
                    painter.add(Shape::line(points, Stroke::new(1.5, Color32::WHITE)));
                }
            }
            _ => {}
        }
    }
    for point in &app.piano_anchor.candidates {
        draw_point(point);
    }
    if let Some(Gesture::Anchor(gesture)) = &app.gesture
        && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
        && let Some(rect) = gesture.selection_rect(pointer)
    {
        painter.rect_filled(rect, 0, GREEN.gamma_multiply(0.12));
        painter.rect_stroke(rect, 0, Stroke::new(1.0, GREEN), StrokeKind::Inside);
    }
}
