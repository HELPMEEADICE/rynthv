#![cfg_attr(
    all(windows, not(feature = "cli"), not(debug_assertions)),
    windows_subsystem = "windows"
)]

#[cfg(feature = "gui")]
mod app;
#[cfg(feature = "gui")]
mod ui;

#[cfg(feature = "gui")]
use rynthv::{editor, midi, model, storage, synthesis};

fn main() -> std::process::ExitCode {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first
        .as_deref()
        .is_some_and(|arg| arg == "-cli" || arg == "--cli")
    {
        #[cfg(feature = "cli")]
        return rynthv::cli::run(args);
        #[cfg(not(feature = "cli"))]
        {
            eprintln!("RynthV was built without the cli feature.");
            return std::process::ExitCode::from(2);
        }
    }

    #[cfg(feature = "gui")]
    {
        let result = launch_gui(first.map(std::path::PathBuf::from));
        if let Err(error) = result {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
        std::process::ExitCode::SUCCESS
    }
    #[cfg(not(feature = "gui"))]
    {
        eprintln!("RynthV was built for headless use. Use rynthv -cli --help.");
        std::process::ExitCode::from(2)
    }
}

#[cfg(feature = "gui")]
fn launch_gui(path: Option<std::path::PathBuf>) -> Result<(), String> {
    #[cfg(all(windows, feature = "cli", not(debug_assertions)))]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const GUI_CHILD: &str = "RYNTHV_GUI_CHILD";
        if std::env::var_os(GUI_CHILD).is_none() {
            // Console subsystem gives shells the correct CLI wait and handle
            // inheritance. Start the GUI in a detached process so it can
            // return control to the launching terminal immediately.
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            std::process::Command::new(executable)
                .args(path)
                .env(GUI_CHILD, "1")
                .creation_flags(DETACHED_PROCESS)
                .spawn()
                .map_err(|error| error.to_string())?;
            return Ok(());
        }
    }
    run_gui(path).map_err(|error| error.to_string())
}

#[cfg(feature = "gui")]
fn run_gui(path: Option<std::path::PathBuf>) -> eframe::Result {
    let mut gpu = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    #[cfg(windows)]
    {
        gpu.instance_descriptor.backends = wgpu::Backends::DX12;
        gpu.instance_descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::Fxc;
    }
    #[cfg(target_os = "linux")]
    {
        gpu.instance_descriptor.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!(
                    "../assets/branding/app-icon.png"
                ))
                .expect("embedded app icon must be a valid PNG"),
            )
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([820.0, 560.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(gpu),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "RynthV",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, path)))),
    )
}
