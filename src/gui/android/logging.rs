use std::ffi::{CString, c_char, c_int};

struct AndroidLog;

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

impl log::Log for AndroidLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn || metadata.target().starts_with("rynthv")
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let priority = match record.level() {
            log::Level::Error => 6,
            log::Level::Warn => 5,
            log::Level::Info => 4,
            log::Level::Debug => 3,
            log::Level::Trace => 2,
        };
        if let Ok(message) = CString::new(format!("{}: {}", record.target(), record.args())) {
            // Android copies the message before returning.
            unsafe { __android_log_write(priority, c"RynthV".as_ptr(), message.as_ptr()) };
        }
    }

    fn flush(&self) {}
}

pub(super) fn install() {
    if log::set_logger(&AndroidLog).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}
