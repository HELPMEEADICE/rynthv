#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    rynthv::gui::run_android(app);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
extern "system" fn Java_org_rynthv_app_RynthVActivity_nativeWake(
    _env: *mut core::ffi::c_void,
    _class: *mut core::ffi::c_void,
) {
    rynthv::gui::wake_android();
}
