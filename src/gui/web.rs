use crate::{
    app::web::Startup,
    platform::web::{audio, documents},
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct BrowserEditor(eframe::WebRunner);

#[wasm_bindgen]
impl BrowserEditor {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(eframe::WebRunner::new())
    }

    pub async fn start(&self, canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
        documents::install_drop(&canvas);
        audio::install_audio(&canvas);
        let start = Startup::load().await;
        let mut gpu = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        gpu.instance_descriptor.backends = wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL;
        let options = eframe::WebOptions {
            renderer: eframe::Renderer::Wgpu,
            wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
                wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(gpu),
                ..Default::default()
            },
            ..Default::default()
        };
        self.0
            .start(
                canvas,
                options,
                Box::new(move |context| Ok(Box::new(crate::app::App::new(context, None, start)))),
            )
            .await
    }

    pub fn destroy(&self) {
        self.0.destroy();
    }
}
