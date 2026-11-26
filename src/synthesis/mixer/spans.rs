//! Ordered, nonoverlapping views into immutable owner PCM. Splitting a view
//! adjusts its source offset; replacing a seam never copies whole owner audio.

use super::{Controls, NATIVE_SAMPLE_RATE, RenderedAudio, intersection, mix_samples};
use crate::{tr, trf};
use std::{collections::BTreeMap, ops::Range, sync::Arc};

#[derive(Clone)]
struct Span {
    end: usize,
    audio: Option<Arc<RenderedAudio>>,
    source_start: usize,
}

#[derive(Default)]
pub(super) struct ReadySpans {
    spans: BTreeMap<usize, Span>,
}

impl ReadySpans {
    pub(super) fn insert_audio(
        &mut self,
        audio: Arc<RenderedAudio>,
        coverage: &Range<usize>,
    ) -> Result<Range<usize>, String> {
        if audio.sample_rate() != NATIVE_SAMPLE_RATE {
            return Err(trf!(
                "实时混音要求 48000 Hz PCM，收到 {} Hz",
                audio.sample_rate()
            ));
        }
        let origin = (audio.start_seconds() * f64::from(NATIVE_SAMPLE_RATE)).round();
        if !origin.is_finite() || origin < 0.0 || origin >= isize::MAX as f64 {
            return Err(tr!("PCM 开始位置超出地址范围").into());
        }
        let origin = origin as usize;
        let end = origin
            .checked_add(audio.frames())
            .ok_or(tr!("PCM 结束位置溢出"))?;
        let range = intersection(&(origin..end), coverage);
        if !range.is_empty() {
            self.replace(
                range.clone(),
                Span {
                    end: range.end,
                    audio: Some(audio),
                    source_start: range.start - origin,
                },
            );
        }
        Ok(range)
    }

    pub(super) fn insert_silence(&mut self, range: Range<usize>) {
        if !range.is_empty() {
            self.replace(
                range.clone(),
                Span {
                    end: range.end,
                    audio: None,
                    source_start: 0,
                },
            );
        }
    }

    fn replace(&mut self, range: Range<usize>, span: Span) {
        let first = self
            .spans
            .range(..=range.start)
            .next_back()
            .filter(|(_, span)| span.end > range.start)
            .map_or(range.start, |(start, _)| *start);
        let keys = self
            .spans
            .range(first..range.end)
            .map(|(key, _)| *key)
            .collect::<Vec<_>>();
        for start in keys {
            let old = self
                .spans
                .remove(&start)
                .expect("collected span remains present");
            if start < range.start {
                self.spans.insert(
                    start,
                    Span {
                        end: range.start,
                        ..old.clone()
                    },
                );
            }
            if old.end > range.end {
                self.spans.insert(
                    range.end,
                    Span {
                        source_start: old.source_start + range.end - start,
                        ..old
                    },
                );
            }
        }
        self.spans.insert(range.start, span);
    }

    fn intersecting(&self, range: Range<usize>) -> impl Iterator<Item = (usize, &Span)> {
        let start = self
            .spans
            .range(..=range.start)
            .next_back()
            .filter(|(_, span)| span.end > range.start)
            .map_or(range.start, |(start, _)| *start);
        // Empty intersections can have an end before the start. Normalize the
        // BTree range instead of asking it to construct a reversed range.
        self.spans
            .range(start..range.end.max(start))
            .map(|(start, span)| (*start, span))
    }

    pub(super) fn first_missing(&self, range: Range<usize>) -> Option<usize> {
        let mut cursor = range.start;
        for (start, span) in self.intersecting(range.clone()) {
            if cursor < start {
                break;
            }
            cursor = cursor.max(span.end);
            if cursor >= range.end {
                return None;
            }
        }
        (cursor < range.end).then_some(cursor)
    }

    pub(super) fn has_audio(&self, range: Range<usize>) -> bool {
        self.intersecting(range)
            .any(|(_, span)| span.audio.is_some())
    }

    pub(super) fn mix_into(&self, range: &Range<usize>, controls: &Controls, output: &mut [f32]) {
        for (start, span) in self.intersecting(range.clone()) {
            let Some(audio) = &span.audio else {
                continue;
            };
            let overlap = intersection(&(start..span.end), range);
            let source_start = span.source_start + overlap.start - start;
            let output_start = overlap.start - range.start;
            let frames = overlap.end - overlap.start;
            let channels = audio.layout().channels();
            let (left, right) = controls.gains(audio.layout());
            let input =
                &audio.samples()[source_start * channels..(source_start + frames) * channels];
            let destination = &mut output[output_start * 2..(output_start + frames) * 2];
            mix_samples(input, channels, destination, left, right);
        }
    }
}
