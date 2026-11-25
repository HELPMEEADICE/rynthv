use eframe::egui::{Context, FontData, FontDefinitions, FontFamily};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(target_arch = "wasm32"))]
pub fn install(ctx: &Context) -> Result<(), String> {
    let mut fonts = FontDefinitions::default();
    let result = load(&mut fonts);
    set_definitions(ctx, fonts);
    result
}

#[cfg(target_arch = "wasm32")]
pub fn install_web(ctx: &Context, font: Result<Vec<u8>, String>) -> Result<(), String> {
    let mut fonts = FontDefinitions::default();
    let result = font.map(|bytes| {
        let font = std::sync::Arc::new(FontData::from_owned(bytes));
        register(&mut fonts, font.clone());
        fonts.font_data.insert("ChineseBold".into(), font);
    });
    set_definitions(ctx, fonts);
    result
}

fn set_definitions(ctx: &Context, mut fonts: FontDefinitions) {
    // Register named families even if a system font is missing, so the error
    // can be displayed instead of panicking on the first heading or clock.
    let mut heading = fonts.families[&FontFamily::Proportional].clone();
    if fonts.font_data.contains_key("ChineseBold") {
        heading.insert(0, "ChineseBold".into());
    }
    fonts
        .families
        .insert(FontFamily::Name("Transport".into()), heading.clone());
    fonts
        .families
        .insert(FontFamily::Name("SectionTitle".into()), heading);
    ctx.set_fonts(fonts);
}

#[cfg(not(target_arch = "wasm32"))]
fn load(fonts: &mut FontDefinitions) -> Result<(), String> {
    let regular = std::sync::Arc::new(system_font(false)?);
    #[cfg(target_os = "android")]
    let bold = regular.clone();
    register(fonts, regular);
    #[cfg(not(target_os = "android"))]
    let bold = std::sync::Arc::new(system_font(true)?);
    fonts.font_data.insert("ChineseBold".into(), bold);
    Ok(())
}

fn register(fonts: &mut FontDefinitions, regular: std::sync::Arc<FontData>) {
    fonts.font_data.insert("Chinese".into(), regular);
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Chinese".into());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("Chinese".into());
}

#[cfg(not(target_arch = "wasm32"))]
fn read_font(path: &Path, index: u32) -> Result<FontData, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Cannot load UI font {}: {error}", path.display()))?;
    Ok(FontData {
        index,
        ..FontData::from_owned(bytes)
    })
}

#[cfg(target_os = "windows")]
fn system_font(bold: bool) -> Result<FontData, String> {
    let root = std::env::var_os("WINDIR").ok_or("缺少 WINDIR，无法加载中文字体")?;
    let path = std::path::PathBuf::from(root).join("Fonts").join(if bold {
        "msyhbd.ttc"
    } else {
        "msyh.ttc"
    });
    read_font(&path, 0)
}

#[cfg(target_os = "linux")]
fn system_font(bold: bool) -> Result<FontData, String> {
    linux::load(bold)
}

#[cfg(target_os = "android")]
fn system_font(_bold: bool) -> Result<FontData, String> {
    read_font(Path::new("/system/fonts/NotoSansCJK-Regular.ttc"), 2)
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "android",
    target_arch = "wasm32"
)))]
fn system_font(_bold: bool) -> Result<FontData, String> {
    Err("UI system font loading is supported on Windows and Linux only".into())
}
