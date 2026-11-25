#![cfg_attr(
    all(windows, not(feature = "cli"), not(debug_assertions)),
    windows_subsystem = "windows"
)]

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

    #[cfg(all(feature = "gui", not(target_os = "android")))]
    {
        let result = launch_gui(first.map(std::path::PathBuf::from));
        if let Err(error) = result {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
        std::process::ExitCode::SUCCESS
    }
    #[cfg(not(all(feature = "gui", not(target_os = "android"))))]
    {
        eprintln!("RynthV was built for headless use. Use rynthv -cli --help.");
        std::process::ExitCode::from(2)
    }
}

#[cfg(all(feature = "gui", not(target_os = "android")))]
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
    rynthv::gui::run_desktop(path).map_err(|error| error.to_string())
}
