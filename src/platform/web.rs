use std::{cell::RefCell, future::Future, rc::Rc};
use wasm_bindgen::JsValue;
#[allow(deprecated)]
use wasm_bindgen_futures::{future_to_promise, jspi_block_on_promise};

pub(crate) mod audio;
pub(crate) mod client;
pub(crate) mod documents;
pub(crate) mod fs;
pub(crate) mod protocol;
mod worker;

/// Park the synthesis worker's WASM stack until a browser operation completes.
/// Calls must originate at a JSPI export, with one active job per worker.
#[allow(deprecated)]
pub(crate) fn wait<T: 'static>(future: impl Future<Output = T> + 'static) -> Result<T, String> {
    let result = Rc::new(RefCell::new(None));
    let slot = Rc::clone(&result);
    let promise = future_to_promise(async move {
        slot.replace(Some(future.await));
        Ok(JsValue::UNDEFINED)
    });
    jspi_block_on_promise(&promise).map_err(js_error)?;
    result
        .take()
        .ok_or_else(|| "Browser operation returned no result".into())
}

pub(crate) fn js_error(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}
