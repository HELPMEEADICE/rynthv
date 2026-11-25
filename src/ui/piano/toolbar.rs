use crate::{
    app::App,
    ui::{
        icons::{Icon, IconButton},
        theme::*,
    },
};
use eframe::egui::*;

#[cfg(not(target_os = "android"))]
pub fn toolbar(app: &mut App, ui: &mut Ui) {
    ui.style_mut().always_scroll_the_only_direction = true;
    ScrollArea::horizontal()
        .id_salt("piano-toolbar-scroll")
        .scroll_bar_visibility(scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, false])
        .show(ui, |ui| contents(app, ui));
}

#[cfg(not(target_os = "android"))]
fn contents(app: &mut App, ui: &mut Ui) {
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        title(app, ui);
        editing_tools(app, ui, 24.0);
        ui.add_space(15.0);
        overlays(app, ui, 24.0);
        super::scale_guide::toggle(app, ui);
        ui.add_space(7.0);
        ui.label(RichText::new("网格").color(MUTED));
        ui.add_space(8.0);
        grid_selector(app, ui, 96.0);
        ui.add_space(9.0);
        snap(app, ui, 24.0);
        crate::ui::scale::controls::toolbar(app, ui);
    });
}

fn overlays(app: &mut App, ui: &mut Ui, height: f32) {
    for (icon, label, active) in [
        (Icon::Pitch, "显示音高曲线", &mut app.show_pitch),
        (Icon::Waveform, "显示渲染波形", &mut app.show_waveform),
        (Icon::Retake, "显示 AI 重录小部件", &mut app.show_retakes),
    ] {
        if ui
            .add(
                IconButton::new(icon, label)
                    .framed()
                    .size(vec2(32.0, height))
                    .selected(*active),
            )
            .clicked()
        {
            *active = !*active;
        }
        ui.add_space(7.0);
    }
}

fn grid_selector(app: &mut App, ui: &mut Ui, width: f32) {
    ComboBox::from_id_salt("grid")
        .width(width)
        .selected_text(format!(
            "{} 分{}",
            app.view.grid,
            if app.view.triplet {
                "三连音"
            } else {
                "音符"
            }
        ))
        .show_ui(ui, |ui| {
            for value in [4, 8, 16, 32, 64, 128] {
                if ui
                    .selectable_label(
                        app.view.grid == value && !app.view.triplet,
                        format!("{value} 分音符"),
                    )
                    .clicked()
                {
                    app.view.grid = value;
                    app.view.triplet = false;
                }
            }
            for value in [8, 16, 32, 64] {
                if ui
                    .selectable_label(
                        app.view.grid == value && app.view.triplet,
                        format!("{value} 分三连音"),
                    )
                    .clicked()
                {
                    app.view.grid = value;
                    app.view.triplet = true;
                }
            }
        });
}

fn snap(app: &mut App, ui: &mut Ui, height: f32) {
    if ui
        .add(
            IconButton::new(Icon::Magnet, "对齐网格")
                .size(vec2(32.0, height))
                .framed()
                .selected(app.view.snap),
        )
        .clicked()
    {
        app.view.snap = !app.view.snap;
    }
}

#[cfg(not(target_os = "android"))]
fn title(app: &mut App, ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(110.0, 28.0), Sense::hover());
    let mut header = crate::ui::child(ui, rect, "piano-title");
    header.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(4.0);
        if ui
            .add(
                IconButton::new(
                    if app.piano_collapsed {
                        Icon::ChevronRight
                    } else {
                        Icon::ChevronDown
                    },
                    "展开 / 折叠钢琴卷帘",
                )
                .size(vec2(20.0, 24.0))
                .icon_size(12.0),
            )
            .clicked()
        {
            app.commit_lyric();
            app.piano_collapsed = !app.piano_collapsed;
            ui.ctx().request_repaint();
        }
        ui.add_space(3.0);
        crate::ui::section_title(ui, "钢琴卷帘", crate::ui::state::DragPayload::Piano);
    });
}

const TOOLS: [(Icon, &str, super::Tool); 4] = [
    (
        Icon::NoteSelect,
        "编辑音符（Alt+1）",
        super::Tool::NoteSelect,
    ),
    (Icon::NoteDraw, "创建音符（Alt+2）", super::Tool::NoteDraw),
    (
        Icon::PitchAnchor,
        "编辑音高控制点（Alt+3）",
        super::Tool::PitchAnchor,
    ),
    (Icon::PitchDraw, "手绘音高（Alt+4）", super::Tool::PitchDraw),
];

fn editing_tools(app: &mut App, ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(160.0, height), Sense::hover());
    ui.painter().rect_filled(rect, 3, BUTTON);
    let mut group = crate::ui::child(ui, rect, "piano-editing-tools");
    group.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (icon, label, tool) in TOOLS {
            if ui
                .add(
                    IconButton::new(icon, label)
                        .size(vec2(40.0, height))
                        .grouped()
                        .selected(tool == app.piano_tool),
                )
                .clicked()
            {
                select(app, tool);
            }
        }
    });
}

fn select(app: &mut App, tool: super::Tool) {
    app.commit_lyric();
    app.piano_tool = tool;
    if tool == super::Tool::PitchAnchor {
        app.editor.selected.clear();
    }
    if tool.is_pitch() {
        app.show_pitch = true;
    }
    app.status.clear();
}

#[cfg(target_os = "android")]
pub fn toolbar(app: &mut App, ui: &mut Ui) {
    if app.lyric.is_some() {
        super::lyrics::toolbar(app, ui);
        return;
    }
    let width = ui.max_rect().width();
    ui.horizontal_centered(|ui| {
        const HEIGHT: f32 = 34.0;
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.spacing_mut().interact_size.y = HEIGHT;
        if width >= 272.0 {
            editing_tools(app, ui, HEIGHT);
        } else {
            ui.menu_button("工具", |ui| {
                for (_, label, tool) in TOOLS {
                    if ui.selectable_label(tool == app.piano_tool, label).clicked() {
                        select(app, tool);
                        ui.close();
                    }
                }
            });
        }
        if width >= 536.0 {
            overlays(app, ui, HEIGHT);
            grid_selector(app, ui, 92.0);
            snap(app, ui, HEIGHT);
        }
        let lanes_active = app.view.phonemes || app.view.lanes.iter().any(|lane| lane.visible);
        let params = ui.add(Button::new("参数").selected(lanes_active));
        Popup::menu(&params)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| crate::ui::lane_tabs::show(app, ui));
        ui.menu_button("选项", |ui| {
            if width < 536.0 {
                ui.checkbox(&mut app.show_pitch, "显示音高曲线");
                ui.checkbox(&mut app.show_waveform, "显示渲染波形");
                ui.checkbox(&mut app.show_retakes, "显示 AI 重录小部件");
                ui.checkbox(&mut app.view.snap, "对齐网格");
                grid_selector(app, ui, 130.0);
                ui.separator();
            }
            ui.horizontal(|ui| {
                super::scale_guide::toggle(app, ui);
                ui.label("显示调式音阶");
            });
            ui.set_min_width(190.0);
            crate::ui::scale::controls::panel(app, ui);
        });
    });
}
