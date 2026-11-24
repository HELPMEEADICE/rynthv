use super::{LyricFrontend, explicit_phones, lyric_text};
use crate::{
    model::{Language, PronunciationDictionary},
    synthesis::{plan::NoteInput, voice::Result},
};
use std::collections::HashSet;

type Syllables = Vec<Vec<String>>;
pub(in crate::synthesis::score) type Pronunciation = (Language, Option<Syllables>);

struct Note<'a> {
    input: &'a NoteInput,
    text: &'a str,
    language: Language,
    explicit: bool,
}

pub(in crate::synthesis::score) fn resolve_context<'a>(
    notes: impl Iterator<Item = &'a NoteInput>,
    inherited: Language,
    frontend: &LyricFrontend,
    dictionary: Option<&PronunciationDictionary>,
) -> Result<Vec<Pronunciation>> {
    let mut notes = notes
        .map(|input| {
            let language = frontend.note_language(input, inherited)?;
            Ok(Note {
                input,
                text: lyric_text(input.lyrics.trim()),
                language,
                explicit: explicit_phones(input, language, dictionary).is_some(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    infer_english(&mut notes, frontend);
    let mut result: Vec<_> = notes.iter().map(|note| (note.language, None)).collect();
    let mut index = 0;
    while let Some(note) = notes.get(index) {
        if note.explicit {
            index += 1;
        } else if note.is_japanese() {
            let end = run_end(&notes, index, |note| note.is_japanese());
            let text: Vec<_> = notes[index..end].iter().map(|note| note.text).collect();
            if text.iter().any(|text| text.chars().any(is_kanji)) {
                for (entry, phones) in result[index..end]
                    .iter_mut()
                    .zip(frontend.resolve_japanese_notes(&text)?)
                {
                    entry.1 = Some(phones);
                }
            }
            index = end;
        } else if note.language == Language::English && is_english_word(note.text) {
            let end = run_end(&notes, index, |part| {
                part.language == Language::English
                    && (part.text == "+"
                        || part.text.strip_prefix('-').is_some_and(is_english_word))
            });
            if end > index + 1 {
                let word: String = notes[index..end]
                    .iter()
                    .filter(|note| note.text != "+")
                    .map(|note| note.text.trim_start_matches('-'))
                    .collect();
                if let Ok(syllables) = frontend.resolve(Language::English, &word)
                    && syllables.len() == end - index
                {
                    for (entry, syllable) in result[index..end].iter_mut().zip(syllables) {
                        entry.1 = Some(vec![syllable]);
                    }
                }
            }
            index = end;
        } else {
            index += 1;
        }
    }
    Ok(result)
}

fn infer_english(notes: &mut [Note<'_>], frontend: &LyricFrontend) {
    let english: HashSet<_> = notes
        .iter()
        .filter(|note| note.language == Language::English)
        .map(|note| note.text.to_ascii_lowercase())
        .collect();
    for note in notes {
        if note.language == Language::Japanese
            && !note.explicit
            && note
                .input
                .attributes
                .get("languageOverride")
                .and_then(serde_json::Value::as_str)
                .is_none_or(str::is_empty)
            && is_english_word(note.text)
            && english.contains(&note.text.to_ascii_lowercase())
            && frontend.resolve(Language::Japanese, note.text).is_err()
        {
            note.language = Language::English;
        }
    }
}

fn run_end(notes: &[Note<'_>], start: usize, accepts: impl Fn(&Note<'_>) -> bool) -> usize {
    let mut end = start + 1;
    while let Some(next) = notes.get(end) {
        let previous = notes[end - 1].input;
        if next.explicit
            || previous.onset.0.saturating_add(previous.duration) != next.input.onset.0
            || !accepts(next)
        {
            break;
        }
        end += 1;
    }
    end
}

impl Note<'_> {
    fn is_japanese(&self) -> bool {
        self.language == Language::Japanese
            && !self.text.is_empty()
            && self
                .text
                .chars()
                .all(|ch| is_kanji(ch) || matches!(ch, '\u{3040}'..='\u{30ff}'))
    }
}

fn is_english_word(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || byte == b'\'')
}

fn is_kanji(ch: char) -> bool {
    matches!(ch, '々' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}')
}
