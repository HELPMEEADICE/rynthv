use std::{ffi::c_void, os::windows::ffi::OsStringExt, path::PathBuf, ptr};

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const FOLDERID_ROAMING_APP_DATA: Guid = Guid {
    data1: 0x3EB6_85DB,
    data2: 0x65F9,
    data3: 0x4CF6,
    data4: [0xA0, 0x3A, 0xE3, 0xEF, 0x65, 0x72, 0x9F, 0x3D],
};

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHGetKnownFolderPath(
        rfid: *const Guid,
        flags: u32,
        token: *mut c_void,
        path: *mut *mut u16,
    ) -> i32;
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoTaskMemFree(pv: *mut c_void);
}

struct KnownFolderString(*mut u16);

impl Drop for KnownFolderString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: SHGetKnownFolderPath allocates the wide string with CoTaskMemAlloc.
            unsafe { CoTaskMemFree(self.0.cast()) };
        }
    }
}

pub(super) fn roaming_directory() -> Result<PathBuf, String> {
    let mut raw = ptr::null_mut();
    // SAFETY: FOLDERID_ROAMING_APP_DATA is valid for read; raw receives a CoTaskMemAlloc'd NUL-terminated UTF-16 string on S_OK (0).
    let hr =
        unsafe { SHGetKnownFolderPath(&FOLDERID_ROAMING_APP_DATA, 0, ptr::null_mut(), &mut raw) };
    let guard = KnownFolderString(raw);
    if hr != 0 || guard.0.is_null() {
        return Err("无法定位当前用户的漫游配置目录。".into());
    }
    let mut len = 0usize;
    // SAFETY: SHGetKnownFolderPath returns a valid NUL-terminated UTF-16 string.
    while unsafe { *guard.0.add(len) } != 0 {
        len += 1;
    }
    if len == 0 {
        return Err("当前用户的漫游配置目录路径无效。".into());
    }
    // SAFETY: guard.0 points to len initialized u16 code units.
    let slice = unsafe { std::slice::from_raw_parts(guard.0, len) };
    let path = PathBuf::from(std::ffi::OsString::from_wide(slice));
    if !path.is_absolute() {
        return Err("当前用户的漫游配置目录路径无效。".into());
    }
    Ok(path)
}
