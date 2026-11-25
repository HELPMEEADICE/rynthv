use eframe::egui::*;
mod fonts;
mod palette;
#[cfg(not(target_arch = "wasm32"))]
pub use fonts::install as fonts;
#[cfg(target_arch = "wasm32")]
pub use fonts::install_web as web_fonts;
pub use palette::*;

pub fn section_title_font() -> FontId {
    FontId::new(12.0, FontFamily::Name("SectionTitle".into()))
}

pub fn install(ctx: &Context) {
    #[cfg(target_arch = "wasm32")]
    palette::set_light(false);
    install_style(ctx, Theme::Dark);
}

#[cfg(target_arch = "wasm32")]
pub fn set_light(ctx: &Context, light: bool) {
    let theme = if light { Theme::Light } else { Theme::Dark };
    palette::set_light(light);
    if ctx.theme() == theme {
        return;
    }
    install_style(ctx, theme);
}

fn install_style(ctx: &Context, theme: Theme) {
    let mut style = Style {
        visuals: match theme {
            Theme::Dark => Visuals::dark(),
            Theme::Light => Visuals::light(),
        },
        ..Default::default()
    };
    style.visuals.panel_fill = panel_color();
    style.visuals.window_fill = panel_color();
    style.visuals.extreme_bg_color = background_color();
    style.visuals.override_text_color = Some(text_color());
    style.visuals.selection.bg_fill = match theme {
        Theme::Dark => Color32::from_rgb(72, 91, 48),
        Theme::Light => Color32::from_rgb(191, 220, 170),
    };
    style.visuals.selection.stroke = Stroke::new(1.0, green_color());
    style.visuals.widgets.inactive.bg_fill = button_color();
    style.visuals.widgets.inactive.weak_bg_fill = button_color();
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(0.5, border_color());
    style.visuals.widgets.hovered.bg_fill = button_hover_color();
    style.visuals.widgets.hovered.weak_bg_fill = button_hover_color();
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(0.5, border_color());
    style.visuals.widgets.active.bg_fill = button_active_color();
    style.visuals.widgets.active.weak_bg_fill = button_active_color();
    style.visuals.widgets.active.bg_stroke = Stroke::new(0.5, border_color());
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(2);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(2);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(2);
    style.spacing.item_spacing = vec2(6.0, 6.0);
    style.spacing.button_padding = vec2(7.0, 4.0);
    style.spacing.interact_size.y = 24.0;
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(12.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(12.0));
    ctx.set_theme(theme);
    ctx.set_style_of(theme, style);
}
