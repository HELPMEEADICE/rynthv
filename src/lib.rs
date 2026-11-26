#[cfg(any(feature = "gui", all(target_arch = "wasm32", feature = "web")))]
mod app;
#[cfg(all(feature = "cli", not(target_arch = "wasm32")))]
pub mod cli;
pub mod editor;
#[cfg(any(feature = "gui", all(target_arch = "wasm32", feature = "web")))]
pub mod gui;
pub mod i18n;
pub mod midi;
pub mod model;
pub mod platform;
pub mod storage;
pub mod synthesis;
#[cfg(any(feature = "gui", all(target_arch = "wasm32", feature = "web")))]
mod ui;
