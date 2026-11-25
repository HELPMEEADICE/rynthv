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
    sync::Mutex,
};

thread_local! {
    static ACTIVITY: RefCell<Option<AndroidApp>> = const { RefCell::new(None) };
}
static REPAINT: Mutex<Option<Context>> = Mutex::new(None);

pub(crate) fn install_activity(app: AndroidApp) {
    ACTIVITY.set(Some(app));
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
    f: impl FnOnce(&mut Env<'_>, &Global<JObject<'static>>) -> JniResult<T>,
) -> Result<T, String> {
    ACTIVITY.with_borrow(|app| {
        let app = app.as_ref().ok_or("Android Activity 未初始化")?;
        let vm = JavaVM::singleton().map_err(|error| error.to_string())?;
        let activity = app.activity_as_ptr().cast();
        vm.attach_current_thread(|env| {
            // The AndroidApp owns this global reference for the duration of the call.
            let activity = unsafe { env.as_cast_raw::<Global<JObject>>(&activity)? };
            f(env, activity)
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
    string_method(jni::jni_str!("rynthvFilesDirectory"))?
        .map(PathBuf::from)
        .ok_or_else(|| "Android 无法定位应用私有目录".into())
}

pub(crate) fn finish() -> Result<(), String> {
    with_activity(|env, activity| {
        env.call_method(
            activity,
            jni::jni_str!("rynthvFinish"),
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
