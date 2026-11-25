//! OPFS handles live in the synthesis Worker. Reads and writes use a borrowed
//! view of WASM memory only for the duration of a synchronous handle call.

use super::{js_error, wait};
use js_sys::{Promise, Reflect, Uint8Array};
use serde::Deserialize;
use std::{
    ffi::OsString,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(raw_module = "./opfs.js")]
extern "C" {
    fn open_handle(
        path: &str,
        write: bool,
        create: bool,
        exclusive: bool,
        truncate: bool,
    ) -> Promise;
    #[wasm_bindgen(catch)]
    fn read_handle(id: u32, bytes: &Uint8Array, offset: f64) -> Result<usize, JsValue>;
    #[wasm_bindgen(catch)]
    fn write_handle(id: u32, bytes: &Uint8Array, offset: f64) -> Result<usize, JsValue>;
    #[wasm_bindgen(catch)]
    fn size_handle(id: u32) -> Result<f64, JsValue>;
    #[wasm_bindgen(catch)]
    fn flush_handle(id: u32) -> Result<(), JsValue>;
    fn close_handle(id: u32);
    fn stat_path(path: &str) -> Promise;
    fn list_path(path: &str) -> Promise;
    fn make_directory(path: &str, recursive: bool) -> Promise;
    fn remove_path(path: &str, directory: bool, recursive: bool) -> Promise;
    fn move_path(source: &str, target: &str, replace: bool) -> Promise;
}

fn js_io(error: JsValue) -> io::Error {
    use io::ErrorKind::*;
    let name = Reflect::get(&error, &"name".into())
        .ok()
        .and_then(|name| name.as_string());
    let kind = match name.as_deref() {
        Some("NotFoundError") => NotFound,
        Some("AlreadyExistsError") => AlreadyExists,
        Some("QuotaExceededError") => StorageFull,
        Some("NoModificationAllowedError") => WouldBlock,
        Some("NotAllowedError" | "SecurityError") => PermissionDenied,
        _ => Other,
    };
    io::Error::new(kind, js_error(error))
}

fn resolve(promise: Promise) -> io::Result<JsValue> {
    wait(JsFuture::from(promise))
        .map_err(io::Error::other)?
        .map_err(js_io)
}

fn json<T: serde::de::DeserializeOwned>(value: JsValue) -> io::Result<T> {
    let json = value
        .as_string()
        .ok_or_else(|| io::Error::other("OPFS response is not JSON"))?;
    serde_json::from_str(&json).map_err(io::Error::other)
}

fn resource_path(path: &Path) -> io::Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(part) => {
                let part = part.to_str().filter(|part| !part.contains(['\\', '\0']));
                parts.push(part.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "Invalid OPFS path component")
                })?);
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid OPFS resource path",
                ));
            }
        }
    }
    if parts.first() != Some(&"RynthV") {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "File is outside RynthV browser storage",
        ));
    }
    Ok(format!("/{}", parts.join("/")))
}

#[derive(Clone, Copy)]
pub struct FileType(bool);

impl FileType {
    pub fn is_dir(self) -> bool {
        self.0
    }
    pub fn is_file(self) -> bool {
        !self.0
    }
    pub fn is_symlink(self) -> bool {
        false
    }
}

#[derive(Deserialize)]
pub struct Metadata {
    directory: bool,
    size: u64,
}

impl Metadata {
    pub fn is_file(&self) -> bool {
        self.file_type().is_file()
    }
    pub fn is_dir(&self) -> bool {
        self.file_type().is_dir()
    }
    pub fn len(&self) -> u64 {
        self.size
    }
    pub fn file_type(&self) -> FileType {
        FileType(self.directory)
    }
}

pub struct DirEntry {
    path: PathBuf,
    directory: bool,
}

impl DirEntry {
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }
    pub fn file_name(&self) -> OsString {
        self.path.file_name().unwrap_or_default().to_owned()
    }
    pub fn file_type(&self) -> io::Result<FileType> {
        Ok(FileType(self.directory))
    }
}

pub fn read_dir(path: impl AsRef<Path>) -> io::Result<impl Iterator<Item = io::Result<DirEntry>>> {
    let path = resource_path(path.as_ref())?;
    let entries: Vec<(String, bool)> = json(resolve(list_path(&path))?)?;
    let path = PathBuf::from(path);
    Ok(entries.into_iter().map(move |(name, directory)| {
        Ok(DirEntry {
            path: path.join(name),
            directory,
        })
    }))
}

pub fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    json(resolve(stat_path(&resource_path(path.as_ref())?))?)
}

pub fn try_exists(path: impl AsRef<Path>) -> io::Result<bool> {
    match symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn is_dir(path: impl AsRef<Path>) -> bool {
    symlink_metadata(path).is_ok_and(|m| m.is_dir())
}
pub fn is_file(path: impl AsRef<Path>) -> bool {
    symlink_metadata(path).is_ok_and(|m| m.is_file())
}

pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let path = resource_path(path.as_ref())?;
    resolve(stat_path(&path))?;
    Ok(path.into())
}

pub fn create_dir(path: impl AsRef<Path>) -> io::Result<()> {
    resolve(make_directory(&resource_path(path.as_ref())?, false)).map(drop)
}

pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    resolve(make_directory(&resource_path(path.as_ref())?, true)).map(drop)
}

pub fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    resolve(remove_path(&resource_path(path.as_ref())?, true, false)).map(drop)
}

pub fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    resolve(remove_path(&resource_path(path.as_ref())?, true, true)).map(drop)
}

pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    resolve(remove_path(&resource_path(path.as_ref())?, false, false)).map(drop)
}

pub fn rename(source: impl AsRef<Path>, target: impl AsRef<Path>) -> io::Result<()> {
    move_entry(source.as_ref(), target.as_ref(), true)
}

pub fn publish_new_file(source: &Path, target: &Path) -> io::Result<()> {
    move_entry(source, target, false)
}

fn move_entry(source: &Path, target: &Path, replace: bool) -> io::Result<()> {
    resolve(move_path(
        &resource_path(source)?,
        &resource_path(target)?,
        replace,
    ))
    .map(drop)
}

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    let size = usize::try_from(file.metadata()?.len()).map_err(io::Error::other)?;
    bytes.try_reserve_exact(size).map_err(io::Error::other)?;
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub struct File {
    id: u32,
    offset: u64,
}

impl File {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        OpenOptions::new().open(path)
    }

    pub fn create_new(path: impl AsRef<Path>) -> io::Result<Self> {
        OpenOptions::new().write(true).create_new(true).open(path)
    }

    pub fn metadata(&self) -> io::Result<Metadata> {
        Ok(Metadata {
            directory: false,
            size: size_handle(self.id).map_err(js_io)? as u64,
        })
    }

    pub fn sync_all(&self) -> io::Result<()> {
        flush_handle(self.id).map_err(js_io)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        close_handle(self.id);
    }
}

impl Read for File {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        // SAFETY: read_handle is synchronous, mutates this slice, and retains no view.
        let view = unsafe { Uint8Array::view_mut_raw(bytes.as_mut_ptr(), bytes.len()) };
        let count = read_handle(self.id, &view, self.offset as f64).map_err(js_io)?;
        self.offset += count as u64;
        Ok(count)
    }
}

impl Write for File {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // SAFETY: write_handle reads synchronously and retains no view.
        let view = unsafe { Uint8Array::view(bytes) };
        let count = write_handle(self.id, &view, self.offset as f64).map_err(js_io)?;
        self.offset += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}

impl Seek for File {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let offset = match position {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(delta) => self.offset.checked_add_signed(delta),
            SeekFrom::End(delta) => self.metadata()?.len().checked_add_signed(delta),
        };
        self.offset = offset.filter(|offset| *offset < (1 << 53)).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Invalid OPFS file offset")
        })?;
        Ok(self.offset)
    }
}

#[derive(Default)]
pub struct OpenOptions {
    write: bool,
    create: bool,
    exclusive: bool,
    truncate: bool,
}

impl OpenOptions {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn write(&mut self, value: bool) -> &mut Self {
        self.write = value;
        self
    }
    pub fn create(&mut self, value: bool) -> &mut Self {
        self.create = value;
        self
    }
    pub fn create_new(&mut self, value: bool) -> &mut Self {
        self.exclusive = value;
        self
    }
    pub fn truncate(&mut self, value: bool) -> &mut Self {
        self.truncate = value;
        self
    }

    pub fn open(&self, path: impl AsRef<Path>) -> io::Result<File> {
        let id = resolve(open_handle(
            &resource_path(path.as_ref())?,
            self.write,
            self.create || self.exclusive,
            self.exclusive,
            self.truncate,
        ))?;
        let id = id
            .as_f64()
            .ok_or_else(|| io::Error::other("OPFS handle allocation failed"))?;
        Ok(File {
            id: id as u32,
            offset: 0,
        })
    }
}
