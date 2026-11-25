use super::{Failure, Path, Result, SynthesisSettings, Value, json, voice_failure};
use crate::synthesis::{
    dictionary_import,
    voice::{flat, svpk},
};

pub(super) fn voices(settings: &SynthesisSettings, input: &Path, dry_run: bool) -> Result<Value> {
    if svpk::is_package(input) {
        let voice = svpk::import(input, dry_run).map_err(voice_failure)?;
        return Ok(json!({"dry_run":dry_run,"voices":[voice]}));
    }
    let root = settings.flat_voice_directory()?;
    if dry_run {
        let manifests = flat::inspect_import(input).map_err(voice_failure)?;
        return Ok(json!({"dry_run":true,"destination":root,"voices":manifests}));
    }
    let voices = flat::import(input, &root).map_err(voice_failure)?;
    Ok(json!({"voices":voices}))
}

pub(super) fn dictionaries(
    mut settings: SynthesisSettings,
    input: &Path,
    dry_run: bool,
) -> Result<Value> {
    let staged = dictionary_import::prepare(input, dry_run).map_err(|error| {
        Failure::new(
            "DICTIONARY_RESOURCE",
            format!("{}: {error}", input.display()),
        )
    })?;
    if dry_run {
        return Ok(json!({"dry_run":true}));
    }
    settings.dictionaries = Some(staged.path().to_owned());
    settings
        .save()
        .map_err(|error| Failure::new("CONFIG_WRITE", error))?;
    let directory = staged.keep();
    Ok(json!({"directory":directory,"dry_run":false}))
}
