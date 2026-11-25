//! Native voice preparation, neural synthesis, render scheduling and audio output.
pub mod audio;
#[cfg(any(feature = "gui", all(target_arch = "wasm32", feature = "web")))]
mod context_dialog;
pub mod dictionary_import;
pub mod engine;
pub mod execution;
#[cfg(any(feature = "gui", all(target_arch = "wasm32", feature = "web")))]
pub(crate) mod export_request;
pub mod inference;
pub mod instrumental;
pub mod library;
pub(crate) mod mixer;
mod mixing_options;
pub mod notify;
pub mod offline;
pub mod plan;
#[cfg(feature = "synth-profile")]
pub mod profiling;
pub mod render_cache;
pub mod rendered;
mod resource_archive;
#[cfg(all(feature = "gui", not(target_arch = "wasm32")))]
pub mod runtime;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[path = "runtime_web/mod.rs"]
pub mod runtime;
pub mod score;
pub mod settings;
pub mod stages;
pub mod voice;
