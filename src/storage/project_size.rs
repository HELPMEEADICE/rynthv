use crate::trf;
use std::io::{self, Write};

const MAX_PROJECT_MIB: u64 = 64;
pub(super) const MAX_PROJECT_BYTES: u64 = MAX_PROJECT_MIB * 1024 * 1024;

pub(super) fn check_project_size(bytes: u64) -> Result<(), String> {
    if bytes > MAX_PROJECT_BYTES {
        return Err(trf!("工程超过 {} MiB 文件大小上限", MAX_PROJECT_MIB));
    }
    Ok(())
}

pub(super) struct ProjectWriter<W> {
    writer: W,
    bytes: u64,
}

impl<W: Write> ProjectWriter<W> {
    pub(super) fn new(writer: W) -> Self {
        Self { writer, bytes: 0 }
    }
}

impl<W: Write> Write for ProjectWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        check_project_size(self.bytes.saturating_add(bytes.len() as u64))
            .map_err(io::Error::other)?;
        let written = self.writer.write(bytes)?;
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}
