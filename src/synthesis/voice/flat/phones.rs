use std::io::{Read, Seek};

use super::{FlatArchive, FlatPackage};
use crate::synthesis::voice::{PhoneSet, Result, VoiceError};

const MAX_PHONE_SET_BYTES: usize = 64 * 1024;

pub(super) fn read<R: Read + Seek>(
    archive: &mut FlatArchive<R>,
    package: &FlatPackage,
) -> Result<Vec<PhoneSet>> {
    package
        .resources
        .iter()
        .filter_map(|path| {
            let name = path.strip_prefix("phonesets/")?.strip_suffix(".data")?;
            (!name.contains('/')).then_some((path, name))
        })
        .map(|(path, name)| {
            let table = PhoneSet::parse_flat(&archive.read(path, MAX_PHONE_SET_BYTES)?)
                .map_err(|error| VoiceError::new(format!("Flat phone set {name:?}: {error}")))?;
            if table.name != name {
                return Err(VoiceError::new(format!(
                    "Flat phone set {name:?} declares {:?}",
                    table.name
                )));
            }
            Ok(table)
        })
        .collect()
}
