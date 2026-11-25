mod compute;
mod pages;
mod preferences;
mod shortcuts;
mod widgets;

use crate::{
    app::App,
    ui::{
        child,
        icons::{Icon, IconButton},
        theme::*,
    },
};
use eframe::egui::*;
pub use preferences::Preferences;

const FILL: Color32 = Color32::from_rgb(61, 59, 63);
const DIVIDER: Color32 = Color32::from_rgb(76, 74, 78);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Page {
    #[default]
    General,
    Audio,
    Editing,
    Recording,
    Rendering,
    Shortcuts,
}

impl Page {
    const ALL: [Self; 6] = [
        Self::General,
        Self::Audio,
        Self::Editing,
        Self::Recording,
        Self::Rendering,
        Self::Shortcuts,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::General => "常规",
            Self::Audio => "音频与 MIDI",
            Self::Editing => "编辑",
            Self::Recording => "录制",
            Self::Rendering => "渲染",
            Self::Shortcuts => "快捷键",
        }
    }
}

pub fn show(app: &mut App, ctx: &Context) {
    let id = Id::new("settings-page");
    let mut page = ctx.data_mut(|data| data.get_temp::<Page>(id).unwrap_or_default());
    #[cfg(any(target_os = "android", target_arch = "wasm32"))]
    let size = (ctx.content_rect().size() - Vec2::splat(12.0)).min(vec2(800.0, 610.0));
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    let size = (ctx.content_rect().size() - vec2(40.0, 48.0))
        .min(vec2(800.0, 610.0))
        .max(vec2(240.0, 240.0));
    if size.x < 56.0 || size.y < 34.0 {
        return;
    }
    let before = app.preferences.clone();
    let (fill, divider) = if ctx.theme() == Theme::Light {
        (panel_color(), border_color())
    } else {
        (FILL, DIVIDER)
    };
    let mut close = false;
    let modal = Modal::new(Id::new("settings"))
        .backdrop_color(Color32::from_black_alpha(115))
        .frame(
            Frame::new()
                .fill(fill)
                .corner_radius(4)
                .stroke(Stroke::new(1.0, divider)),
        )
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let short = cfg!(target_os = "android") && size.y < 180.0;
            let header_height = if short { 34.0 } else { 50.0 };
            let header = Rect::from_min_size(rect.min, vec2(rect.width(), header_height));
            let padding = if short {
                vec2(12.0, 2.0)
            } else {
                vec2(18.0, 10.0)
            };
            let mut title = child(ui, header.shrink2(padding), "settings-title");
            title.spacing_mut().interact_size.y = 30.0;
            title.horizontal(|ui| {
                let (icon, _) = ui.allocate_exact_size(Vec2::splat(17.0), Sense::hover());
                Icon::Settings.paint(ui, icon, muted_color());
                ui.label(RichText::new("设置").font(section_title_font()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close = ui
                        .add(
                            IconButton::new(Icon::Close, "关闭设置")
                                .size(Vec2::splat(24.0))
                                .icon_size(13.0),
                        )
                        .clicked();
                });
            });
            ui.painter().hline(
                header.x_range(),
                header.bottom(),
                Stroke::new(1.0, Color32::from_rgb(44, 42, 45)),
            );
            let compact = cfg!(any(target_os = "android", target_arch = "wasm32"))
                && (size.x < 540.0 || short);
            let content_rect = if compact {
                let nav = Rect::from_min_size(
                    header.left_bottom() + vec2(12.0, if short { 2.0 } else { 8.0 }),
                    vec2(size.x - 24.0, if short { 30.0 } else { 42.0 }),
                );
                ComboBox::from_id_salt("mobile-settings-navigation")
                    .width(nav.width() - 10.0)
                    .selected_text(page.label())
                    .show_ui(&mut child(ui, nav, "settings-navigation"), |ui| {
                        for candidate in Page::ALL {
                            ui.selectable_value(&mut page, candidate, candidate.label());
                        }
                    });
                Rect::from_min_max(
                    nav.left_bottom() + vec2(0.0, 4.0),
                    rect.max - vec2(12.0, 12.0),
                )
            } else {
                let nav_width = if size.x >= 680.0 { 188.0 } else { 148.0 };
                let split = rect.left() + nav_width;
                let nav_rect = Rect::from_min_max(
                    pos2(rect.left() + 12.0, header.bottom() + 16.0),
                    pos2(split - 16.0, rect.bottom() - 12.0),
                );
                let mut nav = child(ui, nav_rect, "settings-navigation");
                ScrollArea::vertical()
                    .id_salt("settings-navigation-scroll")
                    .show(&mut nav, |ui| {
                        ui.spacing_mut().item_spacing.y = 12.0;
                        for candidate in Page::ALL {
                            if navigation(ui, candidate, page == candidate).clicked() {
                                page = candidate;
                            }
                        }
                    });
                ui.painter().vline(
                    split,
                    header.bottom() + 8.0..=rect.bottom() - 8.0,
                    Stroke::new(1.0, divider),
                );
                Rect::from_min_max(
                    pos2(split + 20.0, header.bottom() + 18.0),
                    rect.max - vec2(20.0, 18.0),
                )
            };
            if !content_rect.is_positive() {
                return;
            }
            let mut content = child(ui, content_rect, ("settings-content", page));
            content.spacing_mut().item_spacing.y = 8.0;
            content.spacing_mut().interact_size.y = 30.0;
            let control = if ctx.theme() == Theme::Light {
                button_color()
            } else {
                Color32::from_rgb(75, 73, 77)
            };
            content.visuals_mut().widgets.inactive.bg_fill = control;
            content.visuals_mut().widgets.inactive.weak_bg_fill = control;
            content.visuals_mut().widgets.inactive.corner_radius = CornerRadius::same(4);
            ScrollArea::vertical()
                .id_salt(("settings-page-scroll", page))
                .show(&mut content, |ui| {
                    ui.set_width(ui.available_width());
                    if !compact {
                        ui.label(
                            RichText::new(page.label())
                                .font(section_title_font())
                                .size(15.0),
                        );
                        ui.add_space(18.0);
                    }
                    match page {
                        Page::General => pages::general(app, ui),
                        Page::Audio => pages::audio(app, ui),
                        Page::Editing => pages::editing(app, ui),
                        Page::Recording => pages::recording(ui),
                        Page::Rendering => pages::rendering(app, ui),
                        Page::Shortcuts => shortcuts::show(ui),
                    }
                });
        });
    ctx.data_mut(|data| data.insert_temp(id, page));
    if app.preferences != before {
        app.preferences.apply(ctx);
    }
    if close || modal.should_close() {
        app.settings = false;
    }
}

fn navigation(ui: &mut Ui, page: Page, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            4,
            if selected {
                button_active_color()
            } else {
                button_color()
            },
        );
    }
    let icon = Rect::from_center_size(rect.left_center() + vec2(19.0, 0.0), Vec2::splat(18.0));
    match page {
        Page::General => Icon::Settings.paint(ui, icon, muted_color()),
        Page::Editing => Icon::Notes.paint(ui, icon, muted_color()),
        Page::Recording => {
            ui.painter()
                .circle_stroke(icon.center(), 7.0, Stroke::new(1.0, muted_color()));
        }
        Page::Rendering => Icon::Render.paint(ui, icon, muted_color()),
        Page::Audio => widgets::speaker(ui, icon),
        Page::Shortcuts => widgets::keyboard(ui, icon),
    }
    ui.painter().text(
        rect.left_center() + vec2(37.0, 0.0),
        Align2::LEFT_CENTER,
        page.label(),
        if selected {
            section_title_font()
        } else {
            FontId::proportional(12.0)
        },
        text_color(),
    );
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, page.label())
    });
    response.on_hover_cursor(CursorIcon::PointingHand)
}
