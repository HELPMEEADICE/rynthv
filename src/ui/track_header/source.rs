use crate::{
    app::App,
    ui::{icons::Icon, theme::*},
};
use eframe::egui::*;

pub(super) fn show(app: &mut App, ui: &mut Ui, rect: Rect, text_rect: Rect, track_index: usize) {
    let painter = ui
        .painter()
        .with_clip_rect(ui.clip_rect().intersect(text_rect));
    let (track_id, clip_name, voice_name) = {
        let track = &app.editor.project.tracks[track_index];
        if track.is_instrumental() {
            (
                track.id,
                Some(
                    track
                        .audio_clip()
                        .map_or("导入伴奏…", |clip| clip.name())
                        .to_owned(),
                ),
                None,
            )
        } else {
            let voice = track
                .main_ref
                .database
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("未指定歌声")
                .to_owned();
            (track.id, None, Some(voice))
        }
    };
    if let Some(text) = clip_name {
        Icon::Waveform.paint(
            ui,
            Rect::from_center_size(rect.min + vec2(24.0, 39.0), Vec2::splat(12.0)),
            MUTED,
        );
        painter.text(
            rect.min + vec2(35.0, 39.0),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(12.0),
            MUTED,
        );
        let click = Rect::from_min_max(
            rect.min + vec2(16.0, 29.0),
            pos2(text_rect.right(), rect.bottom()),
        );
        if ui
            .interact(
                click,
                Id::new(("track-audio-file", track_id)),
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::Default)
            .on_hover_text("导入 / 替换伴奏音频")
            .clicked()
        {
            app.import_audio(Some(track_id), ui.ctx());
        }
        return;
    }
    Icon::Microphone.paint(
        ui,
        Rect::from_center_size(rect.min + vec2(24.0, 39.0), vec2(12.0, 12.0)),
        MUTED,
    );
    painter.text(
        rect.min + vec2(35.0, 39.0),
        Align2::LEFT_CENTER,
        voice_name.unwrap_or_default(),
        FontId::proportional(12.0),
        MUTED,
    );
    let voice_rect = Rect::from_min_max(
        rect.min + vec2(16.0, 29.0),
        pos2(text_rect.right(), rect.bottom()),
    );
    let voice_response = ui
        .interact(
            voice_rect,
            Id::new(("track_voice", track_id)),
            Sense::click(),
        )
        .on_hover_cursor(CursorIcon::Default)
        .on_hover_text("选择歌声");
    if voice_response.clicked() {
        app.commit_lyric();
        app.editor.select_track(track_id);
    }
    crate::ui::voice_library::popup(app, &voice_response);
}
