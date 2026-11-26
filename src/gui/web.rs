use crate::{
    app::web::Startup,
    platform::web::{audio, documents},
};
use eframe::egui_wgpu::WgpuSetupCreateNew;
#[cfg(feature = "webgpu-renderer")]
use eframe::egui_wgpu::{WgpuSetup, WgpuSetupExisting};
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
        #[cfg(feature = "webgpu-renderer")]
        let wgpu_setup = setup_webgpu().await?;
        #[cfg(not(feature = "webgpu-renderer"))]
        let wgpu_setup = {
            let mut setup = WgpuSetupCreateNew::without_display_handle();
            setup.instance_descriptor.backends = wgpu::Backends::GL;
            setup.into()
        };
        let options = eframe::WebOptions {
            renderer: eframe::Renderer::Wgpu,
            wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
                wgpu_setup,
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

#[cfg(feature = "webgpu-renderer")]
async fn setup_webgpu() -> Result<WgpuSetup, JsValue> {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::BROWSER_WEBGPU;
    setup.power_preference = wgpu::PowerPreference::HighPerformance;
    let instance = wgpu::Instance::new(setup.clone().instance_descriptor);

    // Request the actual adapter before eframe can probe WebGPU without a power preference.
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: setup.power_preference,
            ..Default::default()
        })
        .await
        .map_err(|error| JsError::new(&error.to_string()))?;

    let (device, queue) = adapter
        .request_device(&(setup.device_descriptor)(&adapter))
        .await
        .map_err(|error| JsError::new(&error.to_string()))?;
    Ok(WgpuSetupExisting {
        instance,
        adapter,
        device,
        queue,
    }
    .into())
}
