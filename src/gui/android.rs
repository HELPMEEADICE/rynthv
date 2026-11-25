use crate::{app::App, platform::android};
use android_activity::AndroidApp;

pub fn run_android(app: AndroidApp) {
    android::install_activity(app.clone());
    let mut gpu = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    gpu.instance_descriptor.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
    let options = eframe::NativeOptions {
        android_app: Some(app),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(gpu),
            ..Default::default()
        },
        ..Default::default()
    };
    let result = eframe::run_native(
        "RynthV",
        options,
        Box::new(|cc| {
            android::install_repaint(cc.egui_ctx.clone());
            Ok(Box::new(App::new(cc, None)))
        }),
    );
    if let Err(error) = result {
        let message = format!("RynthV 图形初始化失败：{error}");
        let _ = android::with_activity(|env, activity| {
            let message = env.new_string(&message)?;
            env.call_method(
                activity,
                jni::jni_str!("rynthvShowFatalError"),
                jni::jni_sig!("(Ljava/lang/String;)V"),
                &[(&message).into()],
            )?;
            Ok(())
        });
    }
    android::release_activity();
}

pub fn wake_android() {
    android::wake();
}

pub fn android_event(event: i32) {
    android::lifecycle::event(event);
    android::wake();
}
