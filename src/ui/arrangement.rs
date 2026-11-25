use super::{
    child,
    icons::{Icon, IconButton},
    label,
    theme::*,
    timeline::*,
};
use crate::{app::App, model::*};
use eframe::egui::*;

#[cfg(target_os = "android")]
mod touch;

pub fn show(app: &mut App, ui: &mut Ui) {
    let rect = ui.max_rect();
    if ui.rect_contains_pointer(rect) && ui.input(|input| input.pointer.any_pressed()) {
        super::lanes::parameter_deactivate(app);
        super::phonemes::phoneme_deactivate(app);
    }
    ui.painter().rect_filled(rect, 4, CANVAS);
    let mobile = cfg!(target_os = "android");
    let (gutter, header_height, scrollbar_width) = if mobile {
        (
            (rect.width() * 0.42)
                .clamp(106.0, 200.0)
                .min(rect.width() * 0.65),
            34.0,
            12.0,
        )
    } else {
        (250.0_f32.min(rect.width() * 0.36), 42.0, 0.0)
    };
    let collapsed = !mobile && app.view.arrangement_collapsed;
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), header_height));
    ui.painter().rect_filled(
        header,
        if collapsed {
            CornerRadius::same(4)
        } else {
            CornerRadius {
                nw: 4,
                ne: 4,
                sw: 0,
                se: 0,
            }
        },
        PANEL,
    );
    ui.painter()
        .rect_stroke(rect, 4, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    let header_padding = if mobile { 2.0 } else { 8.0 };
    let mut bar = child(
        ui,
        Rect::from_min_max(
            header.min + vec2(6.0, header_padding),
            pos2(rect.left() + gutter - 6.0, header.bottom() - header_padding),
        ),
        "arrangement-header",
    );
    bar.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        #[cfg(not(target_os = "android"))]
        if ui
            .add(
                IconButton::new(
                    if app.view.arrangement_collapsed {
                        Icon::ChevronRight
                    } else {
                        Icon::ChevronDown
                    },
                    "展开 / 折叠编排",
                )
                .size(vec2(12.0, 24.0))
                .icon_size(12.0),
            )
            .clicked()
        {
            app.view.arrangement_collapsed = !app.view.arrangement_collapsed;
        }
        #[cfg(not(target_os = "android"))]
        super::section_title(ui, "编排", super::state::DragPayload::Arrangement);
        #[cfg(target_os = "android")]
        if gutter >= 104.0 {
            ui.label(RichText::new("编排").font(section_title_font()));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let size = if mobile {
                vec2(28.0, 30.0)
            } else {
                vec2(23.0, 24.0)
            };
            let create = ui.add(
                IconButton::new(Icon::Plus, "创建音轨")
                    .size(size)
                    .icon_size(17.0),
            );
            Popup::menu(&create).show(|ui| super::instrumental::create_menu(app, ui));
            if ui
                .add(
                    IconButton::new(Icon::TempoTracks, "显示 / 隐藏曲速和拍号轨")
                        .size(if mobile { size } else { Vec2::splat(24.0) })
                        .icon_size(17.0)
                        .selected(app.show_time_tracks)
                        .background(PANEL),
                )
                .clicked()
            {
                app.show_time_tracks = !app.show_time_tracks;
            }
        });
    });
    if collapsed {
        return;
    }
    let ruler = Rect::from_min_max(
        pos2(rect.left() + gutter, rect.top()),
        pos2(rect.right() - scrollbar_width, header.bottom()),
    );
    ui.painter().hline(
        header.left()..=ruler.left(),
        header.bottom(),
        Stroke::new(1.0, EDGE),
    );
    let time_height = if app.show_time_tracks {
        super::time_tracks::ROW_HEIGHT * 2.0
    } else {
        0.0
    };
    let body = Rect::from_min_max(
        pos2(ruler.left(), header.bottom()),
        rect.max - vec2(scrollbar_width, 6.0),
    );
    if mobile && (body.height() < 8.0 || body.width() < 16.0) {
        return;
    }
    #[cfg(target_os = "android")]
    let (touching, touch_y) = touch::navigate(app, ui, body, rect.left());
    #[cfg(not(target_os = "android"))]
    let (touching, touch_y) = (false, 0.0);
    let pan = app.view_pan.drag(
        ui,
        super::view_pan::Target::Arrangement,
        Rect::from_min_max(ruler.min, body.max),
        !touching
            && app.gesture.is_none()
            && app.group_gesture.is_none()
            && app.audio_drag.is_none(),
    );
    app.view.arrangement.start = (app.view.arrangement.start
        - pan.x as f64 / app.view.arrangement.beat_width as f64)
        .max(0.0);
    let response = ui.interact(body, Id::new("arrangement-background"), Sense::click());
    let vertical = if touching {
        touch_y
    } else if app.view_pan.is_active() {
        pan.y
    } else {
        scroll(ui, &response, body, &mut app.view.arrangement)
    };
    let scroll_id = Id::new("track-scroll");
    let rows_height =
        app.editor.project.tracks.len() as f32 * super::arrangement_tracks::ROW_HEIGHT;
    let max_scroll = (rows_height + time_height - body.height()).max(0.0);
    let scroll = ui
        .ctx()
        .data(|d| d.get_temp::<f32>(scroll_id).unwrap_or(0.0));
    let scroll = (scroll - vertical).clamp(0.0, max_scroll);
    #[cfg(target_os = "android")]
    let scroll = super::scrollbars::track_rows(
        ui,
        Rect::from_min_max(
            pos2(body.right(), body.top()),
            pos2(rect.right(), body.bottom()),
        ),
        rows_height + time_height,
        max_scroll,
        scroll,
    );
    ui.ctx().data_mut(|d| d.insert_temp(scroll_id, scroll));
    let time_y = body.top() - if mobile { scroll } else { 0.0 };
    let first_track_y = body.top() + time_height - scroll;
    follow_playhead(
        ui,
        Rect::from_min_max(ruler.min, body.max),
        &mut app.view.arrangement,
        app.editor.transport.position,
        !touching
            && app.editor.transport.playing
            && app.gesture.is_none()
            && app.group_gesture.is_none()
            && app.audio_drag.is_none()
            && !app.view_pan.is_active(),
    );
    loop_region::show(
        ui,
        Id::new("arrangement-ruler"),
        ruler,
        &app.view.arrangement,
        &mut app.editor.transport,
        loop_region::RulerOptions {
            marker_step: app.view.snap.then_some(ARRANGEMENT_GRID),
            seek_step: None,
            enabled: !touching
                && !app.view_pan.is_active()
                && app.gesture.is_none()
                && app.group_gesture.is_none()
                && app.audio_drag.is_none(),
        },
    );
    draw_ruler(
        ui,
        ruler,
        &app.view.arrangement,
        &app.editor.project.time,
        ARRANGEMENT_GRID,
    );
    grid(
        ui,
        Rect::from_min_max(
            body.min,
            pos2(
                body.right(),
                (time_y + time_height + rows_height).min(body.bottom()),
            ),
        ),
        &app.view.arrangement,
        &app.editor.project.time,
        ARRANGEMENT_GRID,
    );
    if app.show_time_tracks {
        let row_height = super::time_tracks::ROW_HEIGHT;
        let clip = Rect::from_min_max(pos2(rect.left(), body.top()), body.max);
        let mut time_ui = child(ui, clip, "time-tracks");
        for (index, text) in ["拍号", "曲速"].into_iter().enumerate() {
            let y = time_y + index as f32 * row_height;
            time_ui.painter().rect_filled(
                Rect::from_min_size(pos2(rect.left(), y), vec2(gutter, row_height)),
                0,
                PANEL,
            );
            label(
                &time_ui,
                pos2(rect.left() + 14.0, y + row_height * 0.5),
                text,
                MUTED,
            );
            time_ui
                .painter()
                .hline(clip.x_range(), y + row_height, Stroke::new(1.0, EDGE));
        }
        super::time_tracks::show(
            app,
            &mut time_ui,
            Rect::from_min_size(pos2(body.left(), time_y), vec2(body.width(), time_height)),
        );
    }
    let tracks_rect = Rect::from_min_max(
        pos2(rect.left(), (time_y + time_height).max(body.top())),
        body.max,
    );
    super::arrangement_tracks::show(
        app,
        &mut child(ui, tracks_rect, "tracks"),
        ruler.left(),
        first_track_y,
    );
    ui.painter()
        .vline(ruler.left(), rect.y_range(), Stroke::new(1.0, EDGE));
    super::scrollbars::horizontal(
        ui,
        "arrangement-scrollbar",
        Rect::from_min_max(
            pos2(ruler.left() + 2.0, rect.bottom() - 6.0),
            pos2(body.right() - 2.0, rect.bottom()),
        ),
        &mut app.view.arrangement,
        beats(app.editor.project_end()) + 32.0,
    );
    playhead(
        ui,
        Rect::from_min_max(ruler.min, body.max),
        ruler.bottom(),
        &app.view.arrangement,
        app.editor.transport.position,
    );
}
