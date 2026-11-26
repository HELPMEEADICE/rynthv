//! Interface localisation.
//!
//! The msgid of every entry is the Simplified Chinese source text, so an
//! untranslated string degrades to the language the code was written in.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::HashMap,
    sync::{
        OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

/// The interface languages RynthV ships with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Locale {
    #[default]
    SimplifiedChinese,
    TraditionalChinese,
    English,
    Japanese,
}

impl Locale {
    pub const ALL: [Self; 4] = [
        Self::SimplifiedChinese,
        Self::TraditionalChinese,
        Self::English,
        Self::Japanese,
    ];

    /// Stable identifier persisted in preferences.json.
    pub fn id(self) -> &'static str {
        match self {
            Self::SimplifiedChinese => "zh-CN",
            Self::TraditionalChinese => "zh-TW",
            Self::English => "en",
            Self::Japanese => "ja",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|locale| locale.id() == id)
    }

    /// The language's own name, shown untranslated in the picker.
    pub fn native_name(self) -> &'static str {
        match self {
            Self::SimplifiedChinese => "简体中文",
            Self::TraditionalChinese => "繁體中文",
            Self::English => "English",
            Self::Japanese => "日本語",
        }
    }

    fn index(self) -> u8 {
        match self {
            Self::SimplifiedChinese => 0,
            Self::TraditionalChinese => 1,
            Self::English => 2,
            Self::Japanese => 3,
        }
    }

    fn from_index(index: u8) -> Self {
        match index {
            1 => Self::TraditionalChinese,
            2 => Self::English,
            3 => Self::Japanese,
            _ => Self::SimplifiedChinese,
        }
    }
}

impl Serialize for Locale {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

impl<'de> Deserialize<'de> for Locale {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Ok(Self::parse(&id).unwrap_or_default())
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn locale() -> Locale {
    Locale::from_index(CURRENT.load(Ordering::Relaxed))
}

pub fn set_locale(locale: Locale) {
    CURRENT.store(locale.index(), Ordering::Relaxed);
}

/// One catalogue per interface language, keyed by the Simplified Chinese msgid.
struct Catalog {
    traditional_chinese: HashMap<String, String>,
    english: HashMap<String, String>,
    japanese: HashMap<String, String>,
}

impl Catalog {
    fn translation(&self, locale: Locale, key: &str) -> Option<&str> {
        let map = match locale {
            Locale::SimplifiedChinese => return None,
            Locale::TraditionalChinese => &self.traditional_chinese,
            Locale::English => &self.english,
            Locale::Japanese => &self.japanese,
        };
        map.get(key).map(String::as_str)
    }
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();

fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(|| Catalog {
        traditional_chinese: load(CATALOG_ZH_TW),
        english: load(CATALOG_EN),
        japanese: load(CATALOG_JA),
    })
}

const CATALOG_ZH_TW: &[&str] = &[
    include_str!("../i18n/zh-TW/core_ui.tsv"),
    include_str!("../i18n/zh-TW/ui_panels.tsv"),
    include_str!("../i18n/zh-TW/ui_settings.tsv"),
    include_str!("../i18n/zh-TW/ui_voice.tsv"),
    include_str!("../i18n/zh-TW/ui_edit.tsv"),
    include_str!("../i18n/zh-TW/app_core.tsv"),
    include_str!("../i18n/zh-TW/synth_voice.tsv"),
    include_str!("../i18n/zh-TW/synth_engine.tsv"),
    include_str!("../i18n/zh-TW/editor.tsv"),
    include_str!("../i18n/zh-TW/piano.tsv"),
    include_str!("../i18n/zh-TW/lanes.tsv"),
    include_str!("../i18n/zh-TW/arrange.tsv"),
    include_str!("../i18n/zh-TW/misc.tsv"),
];

const CATALOG_EN: &[&str] = &[
    include_str!("../i18n/en/core_ui.tsv"),
    include_str!("../i18n/en/ui_panels.tsv"),
    include_str!("../i18n/en/ui_settings.tsv"),
    include_str!("../i18n/en/ui_voice.tsv"),
    include_str!("../i18n/en/ui_edit.tsv"),
    include_str!("../i18n/en/app_core.tsv"),
    include_str!("../i18n/en/synth_voice.tsv"),
    include_str!("../i18n/en/synth_engine.tsv"),
    include_str!("../i18n/en/editor.tsv"),
    include_str!("../i18n/en/piano.tsv"),
    include_str!("../i18n/en/lanes.tsv"),
    include_str!("../i18n/en/arrange.tsv"),
    include_str!("../i18n/en/misc.tsv"),
];

const CATALOG_JA: &[&str] = &[
    include_str!("../i18n/ja/core_ui.tsv"),
    include_str!("../i18n/ja/ui_panels.tsv"),
    include_str!("../i18n/ja/ui_settings.tsv"),
    include_str!("../i18n/ja/ui_voice.tsv"),
    include_str!("../i18n/ja/ui_edit.tsv"),
    include_str!("../i18n/ja/app_core.tsv"),
    include_str!("../i18n/ja/synth_voice.tsv"),
    include_str!("../i18n/ja/synth_engine.tsv"),
    include_str!("../i18n/ja/editor.tsv"),
    include_str!("../i18n/ja/piano.tsv"),
    include_str!("../i18n/ja/lanes.tsv"),
    include_str!("../i18n/ja/arrange.tsv"),
    include_str!("../i18n/ja/misc.tsv"),
];

fn load(fragments: &[&str]) -> HashMap<String, String> {
    let mut entries = HashMap::new();
    for fragment in fragments {
        for line in fragment.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('\t') else {
                continue;
            };
            if key.is_empty() {
                continue;
            }
            entries.insert(unescape(key), unescape(value));
        }
    }
    entries
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Translate one msgid into the active interface language.
///
/// The msgid is its own fallback, so a missing entry stays readable.
pub fn tr(key: &'static str) -> &'static str {
    let locale = locale();
    if locale == Locale::SimplifiedChinese {
        return key;
    }
    catalog().translation(locale, key).unwrap_or(key)
}

/// Translate a template and substitute its placeholders.
///
/// Placeholders are {} (next argument) or {0}/{1}/... (explicit index), so a
/// translation may reorder them. Arguments are already-formatted text.
pub fn fill(template: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0usize;
    let mut chars = template.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        let mut digits = String::new();
        let mut closed = false;
        while let Some((_, next_char)) = chars.peek().copied() {
            if next_char == '}' {
                chars.next();
                closed = true;
                break;
            }
            if next_char.is_ascii_digit() {
                digits.push(next_char);
                chars.next();
                continue;
            }
            break;
        }
        if !closed {
            out.push_str(&template[index..index + 1]);
            continue;
        }
        let slot = if digits.is_empty() {
            let slot = next;
            next += 1;
            Some(slot)
        } else {
            digits.parse::<usize>().ok()
        };
        match slot.and_then(|slot| args.get(slot)) {
            Some(arg) => out.push_str(&arg.to_string()),
            None => out.push_str("{}"),
        }
    }
    out
}

/// Translate a template that carries {} placeholders.
#[macro_export]
macro_rules! trf {
    ($key:expr $(, $arg:expr)* $(,)?) => {
        $crate::i18n::fill(
            $crate::i18n::tr($key),
            &[$( &$arg as &dyn ::std::fmt::Display ),*],
        )
    };
}

/// Translate one msgid into the active interface language.
#[macro_export]
macro_rules! tr {
    ($key:expr) => {
        $crate::i18n::tr($key)
    };
}
