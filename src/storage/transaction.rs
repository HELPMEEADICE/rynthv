use super::{LoadedProject, Project, check_project_size, load, save_locked};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::Read,
    path::{Path, PathBuf},
};

/// Content fingerprint used for optimistic writes across processes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(target_os = "android", derive(serde::Serialize, serde::Deserialize))]
pub struct Revision([u8; 32]);

impl Revision {
    pub(super) fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    pub fn read(path: &Path) -> Result<Option<Self>, String> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err(format!("{} is not a regular file", path.display()));
        }
        check_project_size(metadata.len())?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        let mut total = 0;
        loop {
            let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            total += count as u64;
            check_project_size(total)?;
            hash.update(&buffer[..count]);
        }
        Ok(Some(Self(hash.finalize().into())))
    }

    pub fn hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

pub struct ProjectFile {
    path: PathBuf,
    _lock: File,
}

impl Drop for ProjectFile {
    fn drop(&mut self) {
        // Release the lock even if a concurrent spawn briefly inherited the
        // open file description before exec closes its copy.
        let _ = self._lock.unlock();
    }
}

impl ProjectFile {
    pub fn lock(path: &Path) -> Result<Self, String> {
        let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
        let canonical = match fs::canonicalize(path) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let parent = path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                let name = path.file_name().ok_or("Project path must name a file")?;
                fs::canonicalize(parent)
                    .map_err(|error| error.to_string())?
                    .join(name)
            }
            Err(error) => return Err(error.to_string()),
        };
        let name = canonical
            .file_name()
            .ok_or("Project path must name a file")?;
        let mut lock_name = std::ffi::OsString::from(".");
        lock_name.push(name);
        lock_name.push(".rynthv-lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(canonical.with_file_name(lock_name))
            .map_err(|error| error.to_string())?;
        lock.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => {
                format!("PROJECT_CONFLICT: {} is being edited", absolute.display())
            }
            TryLockError::Error(error) => error.to_string(),
        })?;
        let path = if absolute
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            canonical
        } else {
            absolute
        };
        Ok(Self { path, _lock: lock })
    }

    pub fn load(&self) -> Result<LoadedProject, String> {
        load(&self.path)
    }

    pub fn save(&self, project: &Project) -> Result<(), String> {
        save_locked(&self.path, project, true)
    }

    pub fn save_if(
        &self,
        project: &Project,
        expected: Option<Revision>,
    ) -> Result<Revision, String> {
        if Revision::read(&self.path)? != expected {
            return Err(format!(
                "PROJECT_CONFLICT: {} changed; reload before saving",
                self.path.display()
            ));
        }
        save_locked(&self.path, project, expected.is_some())?;
        Revision::read(&self.path)?.ok_or_else(|| "Saved project disappeared".into())
    }
}
