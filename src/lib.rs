#[cfg(feature = "gui")]
mod app;
#[cfg(feature = "cli")]
pub mod cli;
pub mod editor;
#[cfg(feature = "gui")]
pub mod gui;
pub mod midi;
pub mod model;
pub mod platform;
pub mod storage;
pub mod synthesis;
#[cfg(feature = "gui")]
mod ui;
