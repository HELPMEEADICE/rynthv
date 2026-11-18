use std::{env, path::PathBuf, process::Command};

const ICON_SCRIPT: &str = "assets/branding/app-icon.rc";

fn main() {
    println!("cargo::rerun-if-changed=assets/branding/app-icon.ico");
    println!("cargo::rerun-if-changed={ICON_SCRIPT}");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let target = env::var("TARGET").expect("Cargo must provide TARGET");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo build output directory"));
    let (mut compiler, resource) = match env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => {
            let resource = out_dir.join("app-icon.res");
            let mut compiler = find_msvc_tools::find(&target, "rc.exe")
                .expect("building the Windows MSVC app icon requires Windows SDK rc.exe");
            compiler
                .arg("/nologo")
                .arg("/fo")
                .arg(&resource)
                .arg(ICON_SCRIPT);
            (compiler, resource)
        }
        Ok("gnu") => {
            let (arch, coff_target) = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
                Ok("x86_64") => ("x86_64", "pe-x86-64"),
                Ok("x86") => ("i686", "pe-i386"),
                _ => panic!("unsupported Windows GNU resource target: {target}"),
            };
            let program = if cfg!(windows) {
                "windres".to_owned()
            } else {
                format!("{arch}-w64-mingw32-windres")
            };
            let resource = out_dir.join("app-icon.o");
            let mut compiler = Command::new(program);
            compiler
                .args(["--target", coff_target, "--output-format", "coff"])
                .arg(ICON_SCRIPT)
                .arg(&resource);
            (compiler, resource)
        }
        _ => panic!("unsupported Windows resource target: {target}"),
    };

    let status = compiler
        .current_dir(root)
        .status()
        .unwrap_or_else(|err| panic!("could not run {:?}: {err}", compiler.get_program()));
    assert!(
        status.success(),
        "app icon resource compilation failed with {status} ({:?})",
        compiler.get_program(),
    );

    // Link the resource only into the editor, not the synthesis command-line tool.
    println!("cargo::rustc-link-arg-bin=rynthv={}", resource.display());
}
