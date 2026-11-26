use super::widgets::*;
use crate::{app::App, tr, trf, ui::state::Column};
use eframe::egui::*;

pub(super) fn general(app: &mut App, ui: &mut Ui) {
    row(ui, tr!("界面语言"), |ui| {
        let mut changed = false;
        ComboBox::from_id_salt("ui-locale")
            .width(ui.available_width().min(180.0))
            .selected_text(app.preferences.locale.native_name())
            .show_ui(ui, |ui| {
                for locale in crate::i18n::Locale::ALL {
                    if ui
                        .selectable_value(&mut app.preferences.locale, locale, locale.native_name())
                        .clicked()
                    {
                        changed = true;
                    }
                }
            });
        if changed {
            crate::i18n::set_locale(app.preferences.locale);
        }
    });
    #[cfg(target_arch = "wasm32")]
    row(ui, tr!("界面主题"), |ui| {
        ui.selectable_value(&mut app.preferences.light_theme, false, tr!("深色"));
        ui.selectable_value(&mut app.preferences.light_theme, true, tr!("浅色"));
    });
    row(ui, tr!("界面缩放"), |ui| {
        let id = ui.make_persistent_id("zoom-draft");
        let mut percent = ui
            .ctx()
            .data_mut(|data| data.get_temp::<f32>(id))
            .unwrap_or(app.preferences.zoom * 100.0);
        ui.label(format!("{percent:.0}%"));
        ui.spacing_mut().slider_width = ui.available_width().max(40.0);
        let response = ui.add(
            Slider::new(&mut percent, 75.0..=200.0)
                .step_by(5.0)
                .show_value(false),
        );
        if response.drag_stopped() || response.changed() && !response.dragged() {
            app.preferences.zoom = percent / 100.0;
        }
        if response.dragged() {
            ui.ctx().data_mut(|data| data.insert_temp(id, percent));
        } else {
            ui.ctx().data_mut(|data| data.remove::<f32>(id));
        }
    });
    fixed(
        ui,
        tr!("图形渲染"),
        &app.graphics_api,
        tr!("编辑器图形渲染后端"),
    );
    section(ui, tr!("帮助和指导"));
    toggle(ui, tr!("显示状态栏"), &mut app.preferences.show_status_bar);
    toggle(
        ui,
        tr!("显示控件操作提示"),
        &mut app.preferences.control_hints,
    );
    section(ui, tr!("工作区"));
    #[cfg(not(target_os = "android"))]
    row(ui, tr!("工具列宽度"), |ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 55.0).max(40.0);
        ui.add(Slider::new(&mut app.view.panel_width, 140.0..=420.0).suffix(" px"));
    });
    row(ui, tr!("布局"), |ui| {
        if ui.button(tr!("重置工作区布局")).clicked() {
            app.view = Default::default();
        }
    });
}

pub(super) fn audio(app: &mut App, ui: &mut Ui) {
    section(ui, tr!("音频输出"));
    fixed(
        ui,
        tr!("输出设备"),
        tr!("系统默认输出设备"),
        tr!("当前跟随系统默认设备，设备切换尚未接入"),
    );
    fixed(
        ui,
        tr!("播放格式"),
        tr!("设备默认格式"),
        tr!("预览音频自动适配输出设备的采样率和声道"),
    );
    if let Some(error) = app.synthesis.playback_error() {
        ui.colored_label(Color32::from_rgb(218, 142, 126), error);
    }
    section(ui, tr!("MIDI 输入"));
    fixed(ui, tr!("输入设备"), tr!("未启用"), tr!("MIDI 输入尚未接入"));
}

pub(super) fn editing(app: &mut App, ui: &mut Ui) {
    section(ui, tr!("钢琴卷帘"));
    row(ui, tr!("半音行高"), |ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 55.0).max(40.0);
        ui.add(Slider::new(&mut app.view.row_height, 12.0..=36.0).suffix(" px"));
    });
    toggle(ui, tr!("音符对齐网格"), &mut app.view.snap);
    row(ui, tr!("网格精度"), |ui| {
        ComboBox::from_id_salt("settings-grid")
            .width(ui.available_width())
            .selected_text(trf!("{} 分音符", app.view.grid))
            .show_ui(ui, |ui| {
                for value in [4, 8, 16, 32, 64, 128] {
                    ui.selectable_value(&mut app.view.grid, value, trf!("{} 分音符", value));
                }
            });
    });
    toggle(ui, tr!("三连音网格"), &mut app.view.triplet);
    section(ui, tr!("显示"));
    toggle(ui, tr!("显示音高线"), &mut app.show_pitch);
    toggle(ui, tr!("显示波形"), &mut app.show_waveform);
    toggle(ui, tr!("编排位于上方"), &mut app.view.arrangement_first);
    toggle(ui, tr!("显示曲速和拍号轨"), &mut app.show_time_tracks);
    ui.add_space(10.0);
    ui.weak(tr!("拖动标题可调整工具列顺序，拖动分隔线可调整区域尺寸。"));
}

pub(super) fn recording(ui: &mut Ui) {
    fixed(
        ui,
        tr!("录音输入设备"),
        tr!("未启用"),
        tr!("录音功能尚未接入"),
    );
    fixed(ui, tr!("录音声道"), tr!("未启用"), tr!("录音功能尚未接入"));
    section(ui, tr!("录制"));
    ui.weak(tr!("录制功能尚未接入，相关设置暂不可用。"));
}

pub(super) fn rendering(app: &mut App, ui: &mut Ui) {
    super::compute::settings(app, ui);
    crate::ui::voice_library::settings(app, ui);
    row(ui, tr!("歌声数据库"), |ui| {
        if ui.button(tr!("管理歌声…")).clicked() {
            app.settings = false;
            app.voice_manager = true;
        }
    });
    section(ui, tr!("音频导出"));
    row(ui, tr!("导出设置"), |ui| {
        if ui.button(tr!("打开渲染面板")).clicked() {
            #[cfg(not(target_os = "android"))]
            if !app.view.columns.contains(&Column::Render) {
                app.view.toggle(Column::Render);
            }
            #[cfg(target_os = "android")]
            crate::ui::mobile::open_panel(app, ui.ctx(), Column::Render);
            app.settings = false;
        }
    });
    ui.weak(tr!(
        "目标文件夹、音轨、声道、位深度和采样率在渲染面板中设置。"
    ));
}
