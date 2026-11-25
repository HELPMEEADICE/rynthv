use std::{fs, io, ops::Deref, path::Path};

pub(super) enum ArchiveBytes {
    Owned(Vec<u8>),
    #[cfg(all(target_os = "android", feature = "gui"))]
    Mapped(memmap2::Mmap),
}

impl ArchiveBytes {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        #[cfg(all(target_os = "android", feature = "gui"))]
        if imported_model(path) {
            let file = fs::File::open(path)?;
            if file.metadata()?.len() >= 0x149 {
                // The SAF importer owns this private subtree. It writes bytes
                // to a new inode, then atomically replaces the document path.
                // An already opened inode is never rewritten or truncated.
                let mapped = unsafe { memmap2::MmapOptions::new().map(&file)? };
                return Ok(Self::Mapped(mapped));
            }
        }
        fs::read(path).map(Self::Owned)
    }
}

impl Deref for ArchiveBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            #[cfg(all(target_os = "android", feature = "gui"))]
            Self::Mapped(bytes) => bytes,
        }
    }
}

#[cfg(all(target_os = "android", feature = "gui"))]
fn imported_model(path: &Path) -> bool {
    crate::platform::config_directory()
        .and_then(|root| {
            root.join("imports/trees")
                .canonicalize()
                .map_err(|error| error.to_string())
        })
        .ok()
        .zip(path.canonicalize().ok())
        .is_some_and(|(root, path)| path.starts_with(root))
}
