use std::collections::BTreeMap;

use crate::{
    model::Language,
    synthesis::voice::{Result, VoiceError}, tr, trf,
};

pub(super) struct Member {
    pub(super) index: usize,
    pub(super) name: String,
    pub(super) size: u64,
    pub(super) directory: bool,
    pub(super) regular: bool,
}

pub(super) fn select(members: &[Member]) -> Result<BTreeMap<String, &Member>> {
    let mut candidates = BTreeMap::<_, Vec<_>>::new();
    for member in members.iter().filter(|member| !member.directory) {
        if let Some((parent, name)) = resource_name(&member.name) {
            candidates.entry(parent).or_default().push((name, member));
        }
    }
    let mut dictionaries = candidates.into_values().filter(|files| {
        Language::ALL.into_iter().any(|language| {
            let contains = |name: &str| {
                files
                    .iter()
                    .any(|(file, member)| file == name && member.size > 0)
            };
            let phone_set = language.phone_set();
            let dictionary = if language == Language::English {
                "cmudict-07b.txt".to_owned()
            } else {
                format!("{phone_set}-dict.txt")
            };
            contains(&dictionary)
                && match language {
                    Language::Mandarin => contains("cedict.txt"),
                    Language::Japanese => [
                        "japanese-romaji-phones.txt",
                        "japanese-hira2romaji-dict.txt",
                        "japanese-kata2romaji-dict.txt",
                        "japanese-sute2romaji-dict.txt",
                        "japanese-kanji-jsm.data",
                    ]
                    .into_iter()
                    .all(contains),
                    Language::Cantonese => {
                        contains("cedict-canto.txt") && contains("cantonese-xsampa-phones.txt")
                    }
                    _ => contains(&format!("{phone_set}-phones.txt")),
                }
        })
    });
    let files = dictionaries
        .next()
        .ok_or_else(|| VoiceError::new(tr!("压缩包中没有完整的 clf-data 语言词典")))?;
    if dictionaries.next().is_some() {
        return Err(VoiceError::new(tr!("压缩包包含多套 clf-data 词典")));
    }
    super::check_size(files.len(), files.iter().map(|(_, member)| member.size))?;
    let mut selected = BTreeMap::new();
    for (name, member) in files {
        if !member.regular || selected.insert(name, member).is_some() {
            return Err(VoiceError::new(trf!("压缩包中的词典条目无效或重名：{0}", member.name)));
        }
    }
    Ok(selected)
}

fn resource_name(path: &str) -> Option<(Vec<&str>, String)> {
    let path = path.strip_prefix("./").unwrap_or(path);
    let mut parts: Vec<_> = path.split(['/', '\\']).collect();
    let name = parts.pop()?.to_ascii_lowercase();
    let (stem, extension) = name.rsplit_once('.')?;
    if parts
        .iter()
        .any(|part| part.is_empty() || matches!(*part, "." | "..") || part.contains(':'))
        || parts
            .last()
            .is_some_and(|part| !part.eq_ignore_ascii_case("clf-data"))
        || !matches!(extension, "txt" | "data")
        || stem.is_empty()
        || stem.len() > 96
        || !stem
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || matches!(stem, "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && matches!(&stem[..3], "com" | "lpt")
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return None;
    }
    Some((parts, name))
}
