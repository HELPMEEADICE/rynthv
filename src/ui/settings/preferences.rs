use eframe::egui::Context;
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub zoom: f32,
    pub show_status_bar: bool,
    pub control_hints: bool,
    #[cfg(target_arch = "wasm32")]
    #[serde(default)]
    pub light_theme: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            show_status_bar: true,
            control_hints: false,
            #[cfg(target_arch = "wasm32")]
            light_theme: false,
        }
    }
}

impl Preferences {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("无法读取界面设置：{error}")),
        };
        let value: Self =
            serde_json::from_slice(&bytes).map_err(|e| format!("无法读取界面设置：{e}"))?;
        if !value.valid() {
            return Err("界面缩放必须为 75% 至 200%".into());
        }
        Ok(value)
    }

    pub fn valid(&self) -> bool {
        self.zoom.is_finite() && (0.75..=2.0).contains(&self.zoom)
    }

    pub fn apply(&self, ctx: &Context) {
        #[cfg(target_arch = "wasm32")]
        crate::ui::theme::set_light(ctx, self.light_theme);
        ctx.set_zoom_factor(self.zoom);
        ctx.style_mut_of(ctx.theme(), |style| {
            style.explanation_tooltips = self.control_hints
        });
    }
}
