//! Decode kana readings with the installed Japanese character transition model.

use crate::platform::fs;
use crate::synthesis::voice::{Result, VoiceError};
use crate::trf;
use std::{collections::HashMap, path::Path};

const HEADER: [u8; 10] = [0, 0, 1, 0, 0, 0, 3, 0, 0, 0];
const BOUNDARY: &str = "#";
const UNSEEN_WEIGHT: f64 = 0.05;

type Weights = HashMap<String, f64>;

pub(super) struct Readings {
    states: HashMap<String, Weights>,
}

struct Candidate {
    state: String,
    reading: String,
    probability: f64,
    previous: usize,
    score: f64,
}

impl Readings {
    pub(super) fn load(path: &Path) -> Result<Self> {
        let bytes =
            fs::read(path).map_err(|e| VoiceError::new(format!("{}：{e}", path.display())))?;
        Self::parse(&bytes)
            .ok_or_else(|| VoiceError::new(trf!("日语读音模型格式无效：{}", path.display())))
    }

    fn parse(bytes: &[u8]) -> Option<Self> {
        // The two model counts follow the version and order. Each table row
        // stores a UTF-8 state, a u16 count and weighted UTF-8 alternatives.
        let mut input = bytes.strip_prefix(&HEADER)?.get(8..)?;
        let mut states = HashMap::new();
        while !input.is_empty() {
            let key = take_text(&mut input)?;
            let (count, rest) = input.split_first_chunk::<2>()?;
            input = rest;
            let count = usize::from(u16::from_le_bytes(*count));
            if count == 0 || count > input.len() / 6 {
                return None;
            }
            let mut weights = HashMap::with_capacity(count);
            for _ in 0..count {
                let value = take_text(&mut input)?;
                let (probability, rest) = input.split_first_chunk::<4>()?;
                input = rest;
                let probability = f64::from(f32::from_le_bytes(*probability));
                if !probability.is_finite() || probability <= 0.0 {
                    return None;
                }
                weights.insert(value.to_owned(), probability);
            }
            if states.insert(key.to_owned(), weights).is_some() {
                return None;
            }
        }
        states.get(BOUNDARY)?;
        Some(Self { states })
    }

    pub(super) fn decode(&self, text: &str) -> Result<Vec<String>> {
        let mut rows: Vec<Vec<Candidate>> = Vec::new();
        for character in text.chars() {
            let mut token = [0; 4];
            let token = character.encode_utf8(&mut token);
            let mut candidates: Vec<_> = if let Some(readings) = self.states.get(token) {
                readings
                    .iter()
                    .filter(|(reading, _)| {
                        reading
                            .chars()
                            .all(|ch| matches!(ch, '\u{3040}'..='\u{30ff}' | '_'))
                    })
                    .map(|(reading, &probability)| Candidate {
                        state: format!("{character},{reading}"),
                        reading: reading.replace('_', ""),
                        probability,
                        previous: 0,
                        score: f64::NEG_INFINITY,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            if candidates.is_empty() {
                return Err(super::unknown(token));
            }
            // Stable ties keep the same pronunciation across runs.
            candidates.sort_unstable_by(|a, b| a.state.cmp(&b.state));
            for candidate in &mut candidates {
                if let Some(previous) = rows.last() {
                    for (index, prior) in previous.iter().enumerate() {
                        let score = prior.score
                            + self.score(&prior.state, &candidate.state, candidate.probability);
                        if score > candidate.score {
                            candidate.score = score;
                            candidate.previous = index;
                        }
                    }
                } else {
                    candidate.score = self.score(BOUNDARY, &candidate.state, candidate.probability);
                }
            }
            rows.push(candidates);
        }
        let Some(last) = rows.last() else {
            return Ok(Vec::new());
        };
        let mut best = last
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                let score = |c: &Candidate| c.score + self.score(&c.state, BOUNDARY, 0.1);
                score(a).total_cmp(&score(b))
            })
            .map(|(index, _)| index)
            .expect("candidate row is nonempty");
        let mut readings = Vec::with_capacity(rows.len());
        for mut row in rows.into_iter().rev() {
            let candidate = row.swap_remove(best);
            readings.push(candidate.reading);
            best = candidate.previous;
        }
        readings.reverse();
        Ok(readings)
    }

    fn score(&self, previous: &str, state: &str, probability: f64) -> f64 {
        let transition = self
            .states
            .get(previous)
            .and_then(|weights| weights.get(state))
            .copied()
            .unwrap_or(0.0);
        (transition + probability * UNSEEN_WEIGHT).ln()
    }
}

fn take_text<'a>(input: &mut &'a [u8]) -> Option<&'a str> {
    let end = input.iter().position(|byte| *byte == 0)?;
    let text = std::str::from_utf8(&input[..end]).ok()?;
    *input = &input[end + 1..];
    (!text.is_empty()).then_some(text)
}
