use super::*;
use std::path::{Path, PathBuf};

const RECORDING_BUDGET: usize = 192 << 20;

#[derive(Default)]
pub(in super::super) struct RecordingCache {
    entries: BTreeMap<PathBuf, (Arc<AudioAsset>, usize, u64)>,
    usage: usize,
    serial: u64,
}

impl RecordingCache {
    pub(in super::super) fn load(
        &mut self,
        path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<Arc<AudioAsset>, String> {
        self.serial += 1;
        if let Some((asset, _, last_use)) = self.entries.get_mut(path) {
            *last_use = self.serial;
            return Ok(Arc::clone(asset));
        }
        let asset = Arc::new(
            AudioAsset::read_with_waveforms(path, cancelled)
                .map_err(|error| format!("AUDIO_RESOURCE: {}: {error}", path.display()))?,
        );
        let size = std::mem::size_of_val(asset.audio.samples())
            + asset
                .waveforms
                .iter()
                .map(|wave| wave.memory_bytes())
                .sum::<usize>();
        if size <= RECORDING_BUDGET {
            while self.usage.saturating_add(size) > RECORDING_BUDGET {
                let Some(path) = self
                    .entries
                    .iter()
                    .min_by_key(|(_, (_, _, used))| used)
                    .map(|(path, _)| path.clone())
                else {
                    break;
                };
                if let Some((_, size, _)) = self.entries.remove(&path) {
                    self.usage -= size;
                }
            }
            self.entries
                .insert(path.to_owned(), (Arc::clone(&asset), size, self.serial));
            self.usage += size;
        }
        Ok(asset)
    }
}
