use super::{Page, current_page, set_page};
use crate::{
    app::App,
    ui::{
        chrome,
        icons::{Icon, IconButton},
        tempo_input, theme,
        time_edit::{self, Target},
    },
};
use eframe::egui::*;

pub(super) fn show(app: &mut App, ui: &mut Ui) {
    let width = ui.max_rect().width();
    let height = ui.max_rect().height() - 2.0;
    ui.painter().rect_filled(ui.max_rect(), 4, theme::PANEL);
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().interact_size.y = height;
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.spacing_mut().button_padding = vec2(7.0, 4.0);
        chrome::mobile_menus(app, ui, width >= 420.0);
        ui.separator();
        transport(app, ui, width, height);
        if width >= 820.0 || app.bpm_edit.is_some() {
            tempo(app, ui);
        }
        ui.separator();
        navigation(app, ui, width);
    });
}

fn navigation(app: &mut App, ui: &mut Ui, width: f32) {
    let ctx = ui.ctx().clone();
    let page = current_page(&ctx);
    if width >= 345.0 {
        for target in Page::EDITORS {
            if ui
                .add(Button::new(target.label()).selected(page == target))
                .clicked()
            {
                set_page(app, &ctx, target);
            }
        }
    }
    if width >= 690.0 {
        for target in Page::PANELS {
            if ui
                .add(Button::new(target.label()).selected(page == target))
                .clicked()
            {
                set_page(app, &ctx, target);
            }
        }
    } else {
        let label = if width < 345.0 || !Page::EDITORS.contains(&page) {
            page.label()
        } else {
            "面板"
        };
        ComboBox::from_id_salt("mobile-page-picker")
            .width(if width < 210.0 { 54.0 } else { 68.0 })
            .selected_text(label)
            .show_ui(ui, |ui| {
                let editors = Page::EDITORS.into_iter().filter(|_| width < 345.0);
                for target in editors.chain(Page::PANELS) {
                    if ui
                        .selectable_label(page == target, target.label())
                        .clicked()
                    {
                        set_page(app, &ctx, target);
                    }
                }
            });
    }
}

fn transport(app: &mut App, ui: &mut Ui, width: f32, height: f32) {
    let button = |icon, label| {
        IconButton::new(icon, label)
            .size(vec2(36.0, height))
            .icon_size(18.0)
    };
    let transport = &mut app.editor.transport;
    let playing = transport.playing;
    if ui
        .add_enabled(
            app.synthesis.recording_error.is_none(),
            if playing {
                button(Icon::Pause, "暂停")
            } else {
                button(Icon::Play, "播放")
            },
        )
        .clicked()
    {
        transport.toggle();
    }
    if width >= 210.0 && ui.add(button(Icon::Stop, "停止")).clicked() {
        transport.stop();
    }
    if width >= 260.0
        && ui
            .add(button(Icon::Loop, "循环播放").selected(transport.looping))
            .clicked()
    {
        transport.looping = !transport.looping;
    }
    if width >= 820.0 {
        if ui.add(button(Icon::Rewind, "跳到开头")).clicked() {
            transport.position = 0;
        }
        if ui.add(button(Icon::Forward, "跳到结尾")).clicked() {
            transport.position = app.editor.project.end();
        }
        let metronome = app.synthesis.metronome_enabled();
        if ui
            .add(button(Icon::Metronome, "节拍器").selected(metronome))
            .clicked()
        {
            app.synthesis.set_metronome(!metronome);
        }
    }
    if width >= 565.0 {
        let seconds = app
            .editor
            .project
            .time
            .seconds_at(transport.position)
            .max(0.0);
        ui.label(format!(
            "{:02}:{:05.2}",
            (seconds / 60.0) as u64,
            seconds % 60.0
        ));
    }
}

fn tempo(app: &mut App, ui: &mut Ui) {
    if app.bpm_edit.is_some() {
        tempo_input::show(app, ui);
    } else {
        let position = app.editor.transport.position;
        let bpm = app.editor.project.time.bpm_at(position);
        if ui.button(format!("{bpm:.2} BPM")).clicked() {
            tempo_input::open(app, position, ui.ctx());
        }
    }
}

pub(in crate::ui) fn transport_options(app: &mut App, ui: &mut Ui) {
    let position = app.editor.transport.position;
    let time = &app.editor.project.time;
    let meter = time.meter_at(time.bar_at(position).0);
    let (tempo_position, bpm) = time
        .tempo
        .iter()
        .rev()
        .find(|event| event.position <= position)
        .map_or((position, time.bpm_at(position)), |event| {
            (event.position, event.bpm)
        });
    if ui
        .button(format!(
            "编辑拍号 {}/{}…",
            meter.numerator, meter.denominator
        ))
        .clicked()
    {
        time_edit::open(app, Target::Meter(meter.index));
        ui.close();
    }
    if ui.button(format!("编辑曲速 {bpm:.2} BPM…")).clicked() {
        time_edit::open(app, Target::Tempo(tempo_position));
        ui.close();
    }
    let mut metronome = app.synthesis.metronome_enabled();
    if ui.checkbox(&mut metronome, "节拍器").changed() {
        app.synthesis.set_metronome(metronome);
    }
    ui.checkbox(
        &mut app.editor.transport.return_on_pause,
        "暂停时返回起播位置",
    );
}
