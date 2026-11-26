mod context;

pub(super) use context::resolve_context;

use super::{LyricFrontend, ScoreNote};
use crate::{
    model::{Language, PronunciationDictionary},
    synthesis::{
        plan::NoteInput,
        voice::{PhoneResolver, Result, VoiceError},
    },
};
use crate::{tr, trf};

pub(super) fn resolve_note(
    note: &NoteInput,
    pronunciation: context::Pronunciation,
    previous: Option<&ScoreNote>,
    frontend: &LyricFrontend,
    dictionary: Option<&PronunciationDictionary>,
    resolver: &PhoneResolver<'_>,
) -> Result<(Language, Vec<Vec<String>>)> {
    let lyrics = note.lyrics.trim();
    let (language, phones) = pronunciation;
    let explicit = explicit_phones(note, language, dictionary);
    let (language, syllables) = if let Some(phones) = explicit {
        (language, split_syllables(phones, language, resolver)?)
    } else {
        let text = lyric_text(lyrics);
        let (language, mut syllables) = match (text, phones) {
            (_, Some(phones)) => (language, phones),
            ("-" | "ー", None) => {
                let (language, vowel) = continuation(previous, resolver)?;
                (language, vec![vec![vowel]])
            }
            ("br", None) => (language, vec![vec!["br".into()]]),
            (text, None) => {
                let text = text.strip_prefix('-').unwrap_or(text);
                let syllables = if language != Language::English
                    && !text.contains(char::is_whitespace)
                    && lyric_text(lyrics).starts_with('-')
                    && resolver
                        .resolve(language.id(), text)
                        .is_ok_and(|phone| !phone.used_silence_fallback)
                {
                    vec![vec![text.to_owned()]]
                } else {
                    frontend.resolve(language, text).map_err(|error| {
                        VoiceError::new(trf!("音符 {} · {}：{}", note.id, language.label(), error))
                    })?
                };
                (language, syllables)
            }
        };
        if text != lyrics
            && let Some(first) = syllables.first_mut()
        {
            first.insert(0, "cl".into());
        }
        (language, syllables)
    };
    if note
        .attributes
        .get("evenSyllableDuration")
        .and_then(serde_json::Value::as_bool)
        == Some(false)
    {
        Ok((language, vec![syllables.into_iter().flatten().collect()]))
    } else {
        Ok((language, syllables))
    }
}

fn explicit_phones<'a>(
    note: &'a NoteInput,
    language: Language,
    dictionary: Option<&'a PronunciationDictionary>,
) -> Option<&'a str> {
    (!note.phonemes.trim().is_empty())
        .then_some(note.phonemes.as_str())
        .or_else(|| {
            dictionary
                .filter(|dict| dict.language == language)
                .and_then(|dict| dict.entries.get(note.lyrics.trim()).map(String::as_str))
        })
}

fn lyric_text(lyrics: &str) -> &str {
    lyrics.trim_start_matches(['\'', '‘', '’'])
}

pub(crate) fn is_continuation(lyrics: &str) -> bool {
    matches!(lyric_text(lyrics.trim()), "-" | "ー")
}

fn continuation(
    previous: Option<&ScoreNote>,
    resolver: &PhoneResolver<'_>,
) -> Result<(Language, String)> {
    if let Some(previous) = previous {
        for phone in previous.phones.iter().rev() {
            let resolved = resolver.resolve(&phone.language, &phone.token)?;
            if matches!(resolved.source_class, Some("vowel" | "diphthong"))
                && let Some(language) = Language::parse(&phone.language)
            {
                return Ok((language, phone.token.clone()));
            }
        }
    }
    Err(VoiceError::new(tr!(
        "延音前缺少相连的元音，请填写歌词或显式音素。"
    )))
}

fn split_syllables(
    phones: &str,
    language: Language,
    resolver: &PhoneResolver<'_>,
) -> Result<Vec<Vec<String>>> {
    let tokens: Vec<String> = phones.split_whitespace().map(str::to_owned).collect();
    let mut nuclei = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let phone = resolver.resolve(language.id(), token)?;
        if matches!(phone.source_class, Some("vowel" | "diphthong")) {
            nuclei.push(index);
        }
    }
    let mut parts = Vec::new();
    let mut start = 0;
    for &end in nuclei.iter().take(nuclei.len().saturating_sub(1)) {
        parts.push(tokens[start..=end].to_vec());
        start = end + 1;
    }
    parts.push(tokens[start..].to_vec());
    Ok(parts)
}
