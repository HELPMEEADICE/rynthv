use clap::Parser;
use std::{ffi::OsString, process::ExitCode};

#[derive(Parser)]
#[command(
    name = "rynthv -cli",
    version,
    about = "RynthV headless vocal production"
)]
struct Arguments {}

pub fn run(args: impl Iterator<Item = OsString>) -> ExitCode {
    match Arguments::try_parse_from(std::iter::once("rynthv -cli".into()).chain(args)) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = error.print();
            ExitCode::from(if error.use_stderr() { 2 } else { 0 })
        }
    }
}
