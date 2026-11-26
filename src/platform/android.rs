use android_activity::AndroidApp;
use eframe::egui::Context;
use jni::{
    Env, JavaVM,
    errors::Result as JniResult,
    objects::{JObject, JString},
    refs::Global,
};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub(crate) mod input;
pub(crate) mod lifecycle;

thread_local! {
    static ACTIVITY: RefCell<Option<AndroidApp>> = const { RefCell::new(None) };
}
static REPAINT: Mutex<Option<Context>> = Mutex::new(None);
static FILES_DIRECTORY: OnceLock<Result<PathBuf, String>> = OnceLock::new();
static CACHE_DIRECTORY: OnceLock<Result<PathBuf, String>> = OnceLock::new();
static TOTAL_MEMORY: OnceLock<u64> = OnceLock::new();

pub(crate) fn install_activity(app: AndroidApp) {
    ACTIVITY.set(Some(app));
    FILES_DIRECTORY.get_or_init(|| {
        string_method(jni::jni_str!("rynthvFilesDirectory"))?
            .map(PathBuf::from)
            .ok_or_else(|| crate::tr!("Android 无法定位应用私有目录").into())
    });
    CACHE_DIRECTORY.get_or_init(|| {
        string_method(jni::jni_str!("rynthvCacheDirectory"))?
            .map(PathBuf::from)
            .ok_or_else(|| crate::tr!("Android 无法定位应用缓存目录").into())
    });
    TOTAL_MEMORY.get_or_init(|| {
        with_activity(|env, activity| {
            env.call_method(
                activity,
                jni::jni_str!("rynthvTotalMemory"),
                jni::jni_sig!("()J"),
                &[],
            )?
            .j()
        })
        .ok()
        .and_then(|value| u64::try_from(value).ok())
        .filter(|&bytes| bytes > 0)
        .unwrap_or(2 * 1024 * 1024 * 1024)
    });
}

pub(crate) fn memory_bytes() -> u64 {
    TOTAL_MEMORY
        .get()
        .copied()
        .unwrap_or(2 * 1024 * 1024 * 1024)
}

pub(crate) fn release_activity() {
    ACTIVITY.set(None);
    if let Ok(mut repaint) = REPAINT.lock() {
        *repaint = None;
    }
}

pub(crate) fn install_repaint(ctx: Context) {
    if let Ok(mut repaint) = REPAINT.lock() {
        *repaint = Some(ctx);
    }
}

pub(crate) fn wake() {
    let ctx = REPAINT.lock().ok().and_then(|repaint| repaint.clone());
    if let Some(ctx) = ctx {
        ctx.request_repaint();
    }
}

pub(crate) fn with_activity<T>(
    f: impl FnOnce(&mut Env<'_>, &JObject<'_>) -> JniResult<T>,
) -> Result<T, String> {
    ACTIVITY.with_borrow(|app| {
        let app = app
            .as_ref()
            .ok_or(crate::tr!("Android Activity 未初始化"))?;
        let vm = JavaVM::singleton().map_err(|error| error.to_string())?;
        let activity = app.activity_as_ptr().cast();
        vm.attach_current_thread(|env| {
            // The AndroidApp owns this global reference for the duration of the call.
            let activity = unsafe { env.as_cast_raw::<Global<JObject>>(&activity)? };
            f(env, &activity)
        })
        .map_err(|error: jni::errors::Error| error.to_string())
    })
}

pub(crate) fn string_method(method: &jni::strings::JNIStr) -> Result<Option<String>, String> {
    with_activity(|env, activity| {
        let value = env
            .call_method(activity, method, jni::jni_sig!("()Ljava/lang/String;"), &[])?
            .l()?;
        if value.is_null() {
            return Ok(None);
        }
        let value = env.cast_local::<JString>(value)?;
        value.try_to_string(env).map(Some)
    })
}

pub(crate) fn files_directory() -> Result<PathBuf, String> {
    FILES_DIRECTORY
        .get()
        .cloned()
        .unwrap_or_else(|| Err(crate::tr!("Android 应用目录未初始化").into()))
}

pub(crate) fn cache_directory() -> Result<PathBuf, String> {
    CACHE_DIRECTORY
        .get()
        .cloned()
        .unwrap_or_else(|| Err(crate::tr!("Android 缓存目录未初始化").into()))
}

pub(crate) fn move_to_background() -> Result<(), String> {
    with_activity(|env, activity| {
        env.call_method(
            activity,
            jni::jni_str!("rynthvMoveToBackground"),
            jni::jni_sig!("()V"),
            &[],
        )?;
        Ok(())
    })
}

pub(crate) fn share(path: &Path, mime: &str) -> Result<(), String> {
    with_activity(|env, activity| {
        let path = env.new_string(path.to_string_lossy())?;
        let mime = env.new_string(mime)?;
        env.call_method(
            activity,
            jni::jni_str!("rynthvShare"),
            jni::jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"),
            &[(&path).into(), (&mime).into()],
        )?;
        Ok(())
    })
}
