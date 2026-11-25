use std::path::Path;

use crate::ui::{icons::Icon, theme::*};
use eframe::egui::*;

/// Artwork is cached by egui's file and image loaders.
pub(super) fn paint(ui: &Ui, rect: Rect, path: Option<&Path>) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.painter()
        .rect_filled(rect, 4, Color32::from_rgb(53, 57, 55));
    if let Some(path) = path
        && let Some(uri) = artwork_uri(ui.ctx(), path)
    {
        Image::from_uri(uri)
            .fit_to_exact_size(rect.size())
            .corner_radius(4)
            .paint_at(ui, rect);
    } else {
        Icon::Microphone.paint(ui, rect.shrink(rect.width() * 0.26), muted_color());
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn artwork_uri(_: &Context, path: &Path) -> Option<String> {
    Some(file_uri(path))
}

#[cfg(target_arch = "wasm32")]
fn artwork_uri(ctx: &Context, path: &Path) -> Option<String> {
    use crate::platform::web::documents;
    let id = Id::new(("voice-artwork", path));
    let uri = format!("bytes://voice-artwork{}", path.display());
    if let Some(loaded) = ctx.data(|data| data.get_temp::<bool>(id)) {
        return loaded.then_some(uri);
    }
    ctx.data_mut(|data| data.insert_temp(id, false));
    let ctx = ctx.clone();
    let path = path.to_owned();
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(bytes) = documents::read_limited(&path, 2 << 20).await {
            ctx.include_bytes(uri, bytes);
            ctx.data_mut(|data| data.insert_temp(id, true));
            ctx.request_repaint();
        }
    });
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn file_uri(path: &Path) -> String {
    #[cfg(windows)]
    {
        let path = path.to_string_lossy().replace('\\', "/");
        if let Some(unc) = path.strip_prefix("//?/UNC/") {
            format!("file://{unc}")
        } else if let Some(local) = path.strip_prefix("//?/") {
            format!("file:///{local}")
        } else if path.starts_with("//") {
            format!("file:{path}")
        } else {
            format!("file:///{path}")
        }
    }
    #[cfg(not(windows))]
    {
        // Unix absolute paths already supply the third slash. Backslashes are
        // valid filename characters, and egui's file loader consumes raw paths.
        format!("file://{}", path.to_string_lossy())
    }
}
