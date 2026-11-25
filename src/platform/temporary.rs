use std::{io, path::Path};

#[cfg(not(target_arch = "wasm32"))]
pub use tempfile::TempDir;

#[cfg(not(target_arch = "wasm32"))]
pub fn create(root: Option<&Path>, prefix: &str) -> io::Result<TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    match root {
        Some(directory) => builder.tempdir_in(directory),
        None => builder.tempdir(),
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use crate::platform::fs;
    use std::path::PathBuf;

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn path(&self) -> &Path {
            &self.0
        }

        pub fn keep(mut self) -> PathBuf {
            std::mem::take(&mut self.0)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            if !self.0.as_os_str().is_empty() {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    pub fn create(root: Option<&Path>, prefix: &str) -> io::Result<TempDir> {
        let root = root.unwrap_or_else(|| Path::new("/RynthV/temporary"));
        fs::create_dir_all(root)?;
        let path = root.join(format!("{prefix}{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path)?;
        Ok(TempDir(path))
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{TempDir, create};
