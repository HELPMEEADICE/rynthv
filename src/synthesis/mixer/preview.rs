use super::{
    Controls, NATIVE_SAMPLE_RATE, intersection, mix_samples, native_bound, spans::ReadySpans,
};
use crate::synthesis::{
    audio::{AudioLayer, RenderedAudio},
    plan::{OwnerId, RenderPlan},
    rendered::patch::PREVIEW_BLOCK_SAMPLES as BLOCK,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::Arc,
};
use crate::{tr};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PreviewSource {
    Vocal(OwnerId),
    Recording(u64),
}

struct Vocal {
    coverage: Range<usize>,
    visible: Range<usize>,
    controls: Controls,
    ready: ReadySpans,
    completed: bool,
}

struct Recording {
    layer: Option<AudioLayer>,
    loaded: bool,
}

pub(crate) struct PreviewMixer {
    blocks: BTreeMap<usize, Vec<PreviewSource>>,
    starts: Vec<(usize, PreviewSource)>,
    vocals: BTreeMap<OwnerId, Vocal>,
    recordings: BTreeMap<u64, Recording>,
    changed: BTreeSet<usize>,
    end: usize,
}

impl PreviewMixer {
    pub(crate) fn new(
        plan: &RenderPlan,
        recordings: impl IntoIterator<Item = (u64, Range<usize>)>,
    ) -> Result<Self, String> {
        let mut mixer = Self {
            blocks: BTreeMap::new(),
            starts: Vec::new(),
            vocals: BTreeMap::new(),
            recordings: BTreeMap::new(),
            changed: BTreeSet::new(),
            end: 0,
        };
        for (&id, request) in &plan.requests {
            let coverage = request.coverage;
            if coverage.sample_rate == 0 {
                return Err(tr!("音符组采样率无效").into());
            }
            let range = native_bound(coverage.sample_start.0, coverage.sample_rate)?
                ..native_bound(coverage.sample_end.0, coverage.sample_rate)?;
            let controls = Controls::read(&plan.output_controls[&id].mixer)?;
            mixer.index(PreviewSource::Vocal(id), &range);
            mixer.vocals.insert(
                id,
                Vocal {
                    coverage: range.clone(),
                    visible: range,
                    controls,
                    ready: ReadySpans::default(),
                    completed: false,
                },
            );
        }
        for (id, range) in recordings {
            mixer.index(PreviewSource::Recording(id), &range);
            mixer.recordings.insert(
                id,
                Recording {
                    layer: None,
                    loaded: false,
                },
            );
        }
        Ok(mixer)
    }

    fn index(&mut self, source: PreviewSource, range: &Range<usize>) {
        self.starts.push((range.start, source));
        if range.is_empty() {
            return;
        }
        self.end = self.end.max(range.end);
        for block in block_indices(range) {
            self.blocks.entry(block).or_default().push(source);
        }
    }

    pub(crate) fn sources(&self) -> &[(usize, PreviewSource)] {
        &self.starts
    }

    pub(crate) fn first_at_or_after(&self, block: usize) -> Option<usize> {
        self.blocks.range(block..).next().map(|(&block, _)| block)
    }

    pub(crate) fn has_changed(&self) -> bool {
        !self.changed.is_empty()
    }

    pub(crate) fn layout(&self) -> (usize, Vec<[usize; 2]>) {
        let mut ranges: Vec<[usize; 2]> = Vec::new();
        for &block in self.blocks.keys() {
            match ranges.last_mut() {
                Some(last) if last[1] == block => last[1] += 1,
                _ => ranges.push([block, block + 1]),
            }
        }
        (self.end, ranges)
    }

    pub(crate) fn duration(&self) -> Option<f64> {
        let end = self
            .vocals
            .values()
            .filter(|vocal| vocal.completed && !vocal.visible.is_empty())
            .map(|vocal| vocal.visible.end)
            .chain(self.recordings.values().filter_map(|recording| {
                recording
                    .layer
                    .as_ref()
                    .map(|layer| layer.start_frame + layer.frames)
            }))
            .max()?;
        (end != 0).then_some(seconds(end))
    }

    pub(crate) fn prepared(&mut self, id: OwnerId, samples: Range<i64>) {
        let vocal = self.vocals.get_mut(&id).expect("preview vocal");
        let samples = samples.start.max(0) as usize..samples.end.max(0) as usize;
        let start = samples
            .start
            .clamp(vocal.coverage.start, vocal.coverage.end);
        let end = samples.end.clamp(start, vocal.coverage.end);
        vocal.visible = start..end;
        for range in [vocal.coverage.start..start, end..vocal.coverage.end] {
            vocal.ready.insert_silence(range.clone());
            self.changed.extend(block_indices(&range));
        }
    }

    pub(crate) fn vocal(&mut self, id: OwnerId, audio: Arc<RenderedAudio>) -> Result<(), String> {
        let vocal = self.vocals.get_mut(&id).expect("preview vocal");
        let range = vocal.ready.insert_audio(audio, &vocal.visible)?;
        self.changed.extend(block_indices(&range));
        Ok(())
    }

    pub(crate) fn cached(&mut self, id: OwnerId, audio: Arc<RenderedAudio>) -> Result<(), String> {
        let start = (audio.start_seconds() * f64::from(NATIVE_SAMPLE_RATE)).round() as i64;
        self.prepared(id, start..start.saturating_add(audio.frames() as i64));
        self.changed
            .extend(block_indices(&self.vocals[&id].visible));
        self.completed(id, audio)
    }

    pub(crate) fn completed(
        &mut self,
        id: OwnerId,
        audio: Arc<RenderedAudio>,
    ) -> Result<(), String> {
        let vocal = self.vocals.get_mut(&id).expect("preview vocal");
        let mut ready = ReadySpans::default();
        ready.insert_silence(vocal.coverage.clone());
        ready.insert_audio(audio, &vocal.visible)?;
        // The cursor already marked each written sample dirty. Replace the
        // progressive spans with a view into the completed shared allocation.
        vocal.ready = ready;
        vocal.completed = true;
        Ok(())
    }

    pub(crate) fn recording(&mut self, id: u64, layer: Option<AudioLayer>, range: Range<usize>) {
        self.recordings.insert(
            id,
            Recording {
                layer,
                loaded: true,
            },
        );
        self.changed.extend(block_indices(&range));
    }

    pub(crate) fn missing(&self, block: usize) -> impl Iterator<Item = (PreviewSource, f64)> {
        let range = block_range(block);
        self.blocks
            .get(&block)
            .into_iter()
            .flatten()
            .filter_map(move |&source| {
                self.first_missing(source, &range)
                    .map(|frame| (source, seconds(frame)))
            })
    }

    pub(crate) fn next_vocal(&self, id: OwnerId) -> f64 {
        let vocal = &self.vocals[&id];
        seconds(
            vocal
                .ready
                .first_missing(vocal.coverage.clone())
                .unwrap_or(vocal.coverage.start),
        )
    }

    pub(crate) fn take_changed(&mut self, demand: usize, budget: usize) -> Vec<usize> {
        let mut blocks = self
            .changed
            .range(demand..)
            .copied()
            .chain(self.changed.range(..demand).copied())
            .take(budget)
            .collect::<Vec<_>>();
        for block in &blocks {
            self.changed.remove(block);
        }
        blocks.retain(|&block| self.missing(block).next().is_none());
        blocks
    }

    /// Publish only complete blocks, in owner order followed by project recording order.
    pub(crate) fn mix(&self, block: usize) -> Option<Vec<f32>> {
        let range = block_range(block);
        let sources = &self.blocks[&block];
        if !sources.iter().any(|source| match source {
            PreviewSource::Vocal(id) => self.vocals[id].ready.has_audio(range.clone()),
            PreviewSource::Recording(id) => {
                self.recordings[id].layer.as_ref().is_some_and(|layer| {
                    !intersection(
                        &range,
                        &(layer.start_frame..layer.start_frame + layer.frames),
                    )
                    .is_empty()
                })
            }
        }) {
            return None;
        }
        let mut output = vec![0.0; range.len() * 2];
        for source in sources {
            match source {
                PreviewSource::Vocal(id) => {
                    let vocal = &self.vocals[id];
                    vocal.ready.mix_into(&range, &vocal.controls, &mut output);
                }
                PreviewSource::Recording(id) => {
                    if let Some(layer) = &self.recordings[id].layer {
                        mix_recording(layer, &range, &mut output);
                    }
                }
            }
        }
        Some(output)
    }

    fn first_missing(&self, source: PreviewSource, range: &Range<usize>) -> Option<usize> {
        match source {
            PreviewSource::Vocal(id) => {
                let vocal = &self.vocals[&id];
                vocal
                    .ready
                    .first_missing(intersection(&vocal.coverage, range))
            }
            PreviewSource::Recording(id) => (!self.recordings[&id].loaded).then_some(range.start),
        }
    }
}

fn mix_recording(layer: &AudioLayer, range: &Range<usize>, output: &mut [f32]) {
    let overlap = intersection(
        range,
        &(layer.start_frame..layer.start_frame + layer.frames),
    );
    if overlap.is_empty() {
        return;
    }
    let channels = layer.audio.layout().channels();
    let source = layer.source_start + overlap.start - layer.start_frame;
    let start = overlap.start - range.start;
    let frames = overlap.len();
    mix_samples(
        &layer.audio.samples()[source * channels..(source + frames) * channels],
        channels,
        &mut output[start * 2..(start + frames) * 2],
        f64::from(layer.gains[0]),
        f64::from(layer.gains[1]),
    );
}

pub(crate) fn block_at(position: f64) -> usize {
    (position * f64::from(NATIVE_SAMPLE_RATE)) as usize / BLOCK
}

fn block_indices(range: &Range<usize>) -> Range<usize> {
    let start = range.start / BLOCK;
    start..if range.is_empty() {
        start
    } else {
        range.end.div_ceil(BLOCK)
    }
}

fn block_range(block: usize) -> Range<usize> {
    block * BLOCK..block.saturating_add(1).saturating_mul(BLOCK)
}

fn seconds(frame: usize) -> f64 {
    frame as f64 / f64::from(NATIVE_SAMPLE_RATE)
}
