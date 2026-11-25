use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

static FOREGROUND: AtomicBool = AtomicBool::new(false);
static AUDIO_FOCUS: AtomicBool = AtomicBool::new(false);
static EVENTS: AtomicU8 = AtomicU8::new(0);
static FOREGROUND_EXPORT: Mutex<Weak<AtomicBool>> = Mutex::new(Weak::new());

pub(crate) const INTERRUPTED: u8 = 1;
pub(crate) const BACK: u8 = 2;
pub(crate) const TRIM_MEMORY: u8 = 4;
pub(crate) const PAUSED: u8 = 8;
pub(crate) const AUDIO_ROUTE: u8 = 16;
pub(crate) const RESUMED: u8 = 32;

pub(crate) fn event(event: i32) {
    let bit = match event {
        0 => {
            FOREGROUND.store(false, Ordering::Release);
            AUDIO_FOCUS.store(false, Ordering::Release);
            if let Ok(export) = FOREGROUND_EXPORT.lock()
                && let Some(export) = export.upgrade()
            {
                export.store(true, Ordering::Release);
            }
            crate::synthesis::runtime::stop_android_stream();
            PAUSED | INTERRUPTED
        }
        1 => {
            FOREGROUND.store(true, Ordering::Release);
            RESUMED
        }
        2 => {
            AUDIO_FOCUS.store(false, Ordering::Release);
            INTERRUPTED
        }
        3 => BACK,
        4 => TRIM_MEMORY,
        5 => AUDIO_ROUTE,
        6 => {
            super::input::ready();
            0
        }
        _ => 0,
    };
    EVENTS.fetch_or(bit, Ordering::Release);
}

pub(crate) fn take_events() -> u8 {
    EVENTS.swap(0, Ordering::AcqRel)
}

pub(crate) fn foreground() -> bool {
    FOREGROUND.load(Ordering::Acquire)
}

pub(crate) fn export_cancellation() -> crate::synthesis::execution::Cancellation {
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut active) = FOREGROUND_EXPORT.lock() {
        flag.store(!foreground(), Ordering::Release);
        *active = Arc::downgrade(&flag);
    }
    crate::synthesis::execution::Cancellation::from_flag(flag)
}

pub(crate) fn audio_allowed() -> bool {
    foreground() && AUDIO_FOCUS.load(Ordering::Acquire)
}

pub(crate) fn request_audio(wanted: bool) -> Result<bool, String> {
    let granted = super::with_activity(|env, activity| {
        env.call_method(
            activity,
            jni::jni_str!("rynthvSetPlaybackWanted"),
            jni::jni_sig!("(Z)Z"),
            &[wanted.into()],
        )?
        .z()
    })?;
    AUDIO_FOCUS.store(wanted && granted, Ordering::Release);
    Ok(granted)
}
