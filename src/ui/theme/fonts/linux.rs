use crate::i18n::Locale;
use eframe::egui::FontData;
use std::{path::Path, process::Command};

const INSTALL_HINT: &str = "Install fontconfig and a CJK sans-serif font (for example fonts-noto-cjk), then restart RynthV.";

/// Fontconfig language tags, in priority order, for one interface language.
fn candidates(locale: Locale) -> &'static [&'static str] {
    match locale {
        Locale::Japanese => &["ja", "zh-tw", "zh-cn"],
        Locale::TraditionalChinese => &["zh-tw", "zh-hk", "zh-cn"],
        Locale::SimplifiedChinese => &["zh-cn", "zh-tw"],
        Locale::English => &["en", "zh-cn"],
    }
}

pub(super) fn load(locale: Locale, bold: bool) -> Result<FontData, String> {
    let weight = if bold { "bold" } else { "regular" };
    let mut last = String::from(INSTALL_HINT);
    for lang in candidates(locale) {
        match find(lang, weight, bold) {
            Ok(font) => return Ok(font),
            Err(message) => last = message,
        }
    }
    Err(last)
}

fn find(lang: &str, weight: &str, bold: bool) -> Result<FontData, String> {
    let output = Command::new("fc-match")
        .args([
            "--format",
            "%{file}\n%{index}\n%{lang}\n",
            &format!("sans-serif:lang={lang}:weight={weight}"),
        ])
        .output()
        .map_err(|error| format!("Cannot run fc-match: {error}. {INSTALL_HINT}"))?;
    if !output.status.success() {
        return Err(format!(
            "Fontconfig could not find a UI font: {}. {INSTALL_HINT}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|error| format!("Invalid fontconfig result: {error}"))?;
    let mut fields = output.lines();
    let path = fields.next().filter(|path| !path.is_empty());
    let index = fields.next().and_then(|index| index.parse::<u32>().ok());
    let languages = fields.next().unwrap_or_default();
    let (Some(path), Some(index)) = (path, index) else {
        return Err(format!("Fontconfig returned no UI font. {INSTALL_HINT}"));
    };
    if !languages.split('|').any(|value| value == lang) {
        return Err(format!(
            "No UI font covers lang={lang}. {INSTALL_HINT}{}",
            if bold { " (bold)" } else { "" }
        ));
    }
    // The upper 16 bits of a Fontconfig face index describe a named variable
    // font instance; egui expects the collection face index only.
    super::read_font(Path::new(path), index & 0xffff)
}
