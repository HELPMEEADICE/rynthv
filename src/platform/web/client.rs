use super::{
    js_error,
    protocol::{Event, Request},
};
use js_sys::{Array, Float32Array, Function, Promise};
use serde::de::DeserializeOwned;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};

#[wasm_bindgen(raw_module = "./browser.js")]
extern "C" {
    fn start_request(request: &str, writes: bool, listener: &Function) -> Array;
    fn cancel_request(id: u32);
}

pub struct Pending<T> {
    id: u32,
    receiver: Receiver<Result<T, String>>,
}

impl<T> Pending<T> {
    pub fn poll(&self) -> Option<Result<T, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("合成 Worker 已停止".into())),
        }
    }

    pub fn cancel(&self) {
        cancel_request(self.id);
    }

    pub fn into_receiver(self) -> Receiver<Result<T, String>> {
        self.receiver
    }
}

pub fn submit<T: DeserializeOwned + 'static>(
    request: Request,
    repaint: crate::synthesis::notify::Notify,
    mut event: impl FnMut(Event, Option<Float32Array>) + 'static,
) -> Pending<T> {
    let writes = matches!(
        &request,
        Request::ImportVoices { .. }
            | Request::ImportDictionaries { .. }
            | Request::Settings { .. }
            | Request::RemoveVoice { .. }
            | Request::Context { .. }
    );
    let request = serde_json::to_string(&request).expect("serializable worker request");
    let listener =
        Closure::<dyn FnMut(String, Option<Float32Array>)>::new(move |message: String, pcm| {
            if let Ok(message) = serde_json::from_str(&message) {
                event(message, pcm);
            }
        });
    let response = start_request(&request, writes, listener.as_ref().unchecked_ref());
    let id = response.get(0).as_f64().expect("worker request id") as u32;
    let promise = response.get(1).unchecked_into::<Promise>();
    let (sender, receiver) = mpsc::channel();
    spawn_local(async move {
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error)
            .and_then(|value| {
                let value = value.as_string().ok_or("合成 Worker 返回了无效回复")?;
                serde_json::from_str(&value).map_err(|error| error.to_string())
            });
        drop(listener);
        let _ = sender.send(result);
        repaint.wake();
    });
    Pending { id, receiver }
}
