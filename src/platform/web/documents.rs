use super::js_error;
use js_sys::{Promise, Uint8Array};
use serde::de::DeserializeOwned;
use std::path::Path;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(raw_module = "./documents.js")]
extern "C" {
    pub fn pick_files(kind: u8, accept: &str, title: &str) -> Promise;
    pub fn install_drop(canvas: &web_sys::HtmlCanvasElement);
    pub fn take_drop() -> Option<String>;
    fn read_document(path: &str, limit: u32) -> Promise;
    pub fn delete_document(path: &str) -> Promise;
    pub fn download_bytes(bytes: &[u8], name: &str, mime: &str);
    pub fn download_file(path: &str) -> Promise;
    pub fn clear_exports() -> Promise;
    fn read_record(key: &str) -> Promise;
    pub fn write_record(key: &str, value: &str) -> Promise;
    pub fn save_project(id: &str, name: &str, snapshot: &str) -> Promise;
    pub fn project_index() -> Promise;
    fn fetch_font() -> Promise;
}

pub async fn json<T: DeserializeOwned>(promise: Promise) -> Result<T, String> {
    let value = JsFuture::from(promise).await.map_err(js_error)?;
    let value = value.as_string().ok_or(crate::tr!("无效的浏览器数据"))?;
    serde_json::from_str(&value).map_err(|error| error.to_string())
}

pub async fn read(path: &Path) -> Result<Vec<u8>, String> {
    read_limited(path, 64 << 20).await
}

pub async fn read_limited(path: &Path, limit: u32) -> Result<Vec<u8>, String> {
    let value = JsFuture::from(read_document(&path.to_string_lossy(), limit))
        .await
        .map_err(js_error)?;
    Ok(Uint8Array::new(&value).to_vec())
}

pub async fn record(key: &str) -> Result<Option<String>, String> {
    JsFuture::from(read_record(key))
        .await
        .map(|value| value.as_string())
        .map_err(js_error)
}

pub async fn font() -> Result<Vec<u8>, String> {
    let value = JsFuture::from(fetch_font()).await.map_err(js_error)?;
    Ok(Uint8Array::new(&value).to_vec())
}
