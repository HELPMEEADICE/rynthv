use crate::{app::App, platform::android};
use android_activity::AndroidApp;
use std::sync::Arc;

mod logging;

pub fn run_android(app: AndroidApp) {
    logging::install();
    android::install_activity(app.clone());
    let mut gpu = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    gpu.instance_descriptor.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
    gpu.native_adapter_selector = Some(Arc::new(|adapters, surface| {
        adapters
            .iter()
            .filter(|adapter| surface.is_none_or(|surface| adapter.is_surface_supported(surface)))
            .min_by_key(|adapter| {
                let info = adapter.get_info();
                (
                    info.device_type == wgpu::DeviceType::Cpu,
                    info.backend != wgpu::Backend::Vulkan,
                )
            })
            .cloned()
            .ok_or_else(|| "No Android Vulkan or OpenGL ES presentation adapter".to_owned())
    }));
    let options = eframe::NativeOptions {
        android_app: Some(app),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(gpu),
            surface: eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY,
            ..Default::default()
        },
        ..Default::default()
    };
    let result = eframe::run_native(
        "RynthV",
        options,
        Box::new(|cc| {
            android::install_repaint(cc.egui_ctx.clone());
            if let Some(state) = &cc.wgpu_render_state {
                monitor_graphics_device(&state.device);
            }
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

fn monitor_graphics_device(device: &wgpu::Device) {
    let activity = android::with_activity(|env, activity| env.new_global_ref(activity));
    let (Ok(activity), Ok(vm)) = (activity, jni::JavaVM::singleton()) else {
        return;
    };
    device.set_device_lost_callback(move |reason, message| {
        if reason == wgpu::DeviceLostReason::Destroyed {
            return;
        }
        let message = format!("Android 图形设备已丢失（{reason:?}）：{message}");
        log::error!("{message}");
        let _: jni::errors::Result<()> = vm.attach_current_thread(|env| {
            let message = env.new_string(&message)?;
            env.call_method(
                &activity,
                jni::jni_str!("rynthvGraphicsLost"),
                jni::jni_sig!("(Ljava/lang/String;)V"),
                &[(&message).into()],
            )?;
            Ok(())
        });
    });
}

pub fn wake_android() {
    android::wake();
}

pub fn android_event(event: i32) {
    android::lifecycle::event(event);
    android::wake();
}
