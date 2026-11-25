use eframe::egui::Color32;

#[cfg(target_arch = "wasm32")]
thread_local! { static LIGHT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

#[cfg(target_arch = "wasm32")]
pub(super) fn set_light(value: bool) {
    LIGHT.set(value);
}

#[inline]
fn choose(dark: Color32, light: Color32) -> Color32 {
    #[cfg(target_arch = "wasm32")]
    if LIGHT.get() {
        return light;
    }
    let _ = light;
    dark
}

macro_rules! colors {
    ($($name:ident: ($dr:literal, $dg:literal, $db:literal) => ($lr:literal, $lg:literal, $lb:literal),)*) => {
        $(#[inline] pub fn $name() -> Color32 {
            choose(Color32::from_rgb($dr, $dg, $db), Color32::from_rgb($lr, $lg, $lb))
        })*
    };
}

colors! {
    background_color: (27, 27, 29) => (238, 238, 241),
    panel_color: (45, 43, 46) => (220, 221, 228),
    canvas_color: (35, 35, 37) => (249, 249, 251),
    row_color: (39, 39, 41) => (246, 246, 249),
    black_row_color: (31, 31, 33) => (228, 229, 235),
    grid_color: (58, 57, 60) => (194, 195, 205),
    text_color: (201, 199, 204) => (42, 43, 54),
    muted_color: (135, 133, 139) => (91, 93, 109),
    green_color: (125, 178, 53) => (62, 113, 18),
    edge_color: (21, 21, 23) => (202, 203, 211),
    border_color: (57, 55, 58) => (174, 177, 190),
    button_color: (68, 66, 71) => (211, 212, 222),
    button_hover_color: (78, 76, 81) => (195, 199, 215),
    button_active_color: (83, 81, 87) => (183, 189, 206),
    icon_tool_color: (181, 179, 186) => (81, 85, 107),
}
