use js_sys::{Float32Array, Float64Array};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(raw_module = "./audio.js")]
extern "C" {
    pub fn install_audio(canvas: &web_sys::HtmlCanvasElement);
    pub fn audio_reset();
    pub fn audio_pcm(seconds: f64, pcm: Float32Array, final_mix: bool);
    pub fn audio_end(seconds: f64);
    pub fn audio_transport(playing: bool, seconds: f64);
    pub fn audio_loop(start: f64, end: f64);
    pub fn audio_clock() -> Option<f64>;
    pub fn audio_error() -> Option<String>;
    fn audio_note(samples: Option<Float32Array>);
    fn audio_clicks(times: Float64Array, strong: Float32Array, weak: Float32Array);
}

pub fn metronome(times: &[f64]) {
    use crate::synthesis::audio::click::click;
    let strong = click(1850.0, 0.24);
    let weak = click(1250.0, 0.16);
    audio_clicks(
        Float64Array::from(times),
        Float32Array::from(strong.as_slice()),
        Float32Array::from(weak.as_slice()),
    );
}

pub fn audition(note: Option<u8>) {
    let samples = note.map(crate::synthesis::audio::audition_pcm);
    audio_note(samples.as_deref().map(Float32Array::from));
}
