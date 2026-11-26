//! Emit the spectral bands and gain from a shared conditional-network result.

mod band_state;

use band_state::BandRows;
pub(crate) use band_state::VocoderBandState;

use super::{ConditionalGraph, MIN_AUTO_WORK, WindowPlan};
use crate::synthesis::{
    inference::{
        Sequence, compute,
        convolution::ConvolutionStream,
        gpu::{Batch, Resident, batch::Kernel, chain::Chain, elementwise},
        matrix::MatrixError,
    },
    voice::{PreparedTree, Result, VoiceError},
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::Arc,
};
use wgpu::Buffer;

#[derive(Debug)]
pub(crate) struct VocoderBand {
    pub node: usize,
    pub start: usize,
    pub end: usize,
    pub weights: Arc<[f32]>,
}

pub(crate) struct VocoderBranches<'a> {
    pub bands: &'a [VocoderBand],
    pub bins: usize,
    pub heads: usize,
    pub gain: usize,
    pub state: &'a mut VocoderBandState,
}

struct BandPlan<'a> {
    chain: Chain<'a>,
    rows: BandRows,
}

struct BranchPlan<'a> {
    bands: Vec<BandPlan<'a>>,
    gain: Chain<'a>,
    capacity: usize,
    frames: usize,
}

struct Updates {
    convolutions: Vec<(usize, ConvolutionStream)>,
    bands: Vec<Option<Resident>>,
}

impl VocoderBranches<'_> {
    pub(in crate::synthesis::inference) fn execute(
        &mut self,
        graph: &ConditionalGraph<'_>,
        tree: &PreparedTree,
        streams: &mut HashMap<usize, ConvolutionStream>,
        input: (&Sequence, Option<&Sequence>),
        rows: Range<usize>,
        flush: bool,
    ) -> Result<Option<(Vec<Sequence>, Sequence)>> {
        let Some(window) = graph.plan(input.0, input.1, &rows)? else {
            return Ok(None);
        };
        let projection =
            |node| Chain::projection(tree, node, rows.len(), graph.channels, streams, flush);
        let mut bands = Vec::with_capacity(self.bands.len());
        let mut nodes = HashSet::new();
        for (spec, previous) in self.bands.iter().zip(&self.state.pending) {
            let Some(chain) = projection(spec.node)? else {
                return Ok(None);
            };
            let expected = spec.weights.len() * self.heads;
            if chain.channels != expected {
                return Err(VoiceError::new(
                    "Spectral branch output has the wrong channel count",
                ));
            }
            if !chain.state_nodes().all(|node| nodes.insert(node)) {
                return Ok(None);
            }
            let rows = BandRows {
                pending: previous.as_ref().map_or(0, Resident::len) / expected,
                fresh: chain.frames,
                width: expected,
            };
            bands.push(BandPlan { chain, rows });
        }
        let Some(gain) = projection(self.gain)? else {
            return Ok(None);
        };
        if gain.channels != 10 {
            return Err(VoiceError::new(
                "Gain branch output has the wrong channel count",
            ));
        }
        if !gain.state_nodes().all(|node| nodes.insert(node)) {
            return Ok(None);
        }

        let frames = bands
            .iter()
            .map(|band| band.rows.ready())
            .min()
            .expect("prepared spectral program contains at least one band");
        if flush && bands.iter().any(|band| band.rows.ready() != frames) {
            return Err(VoiceError::new(
                "Spectral branches ended with unaligned frame counts",
            ));
        }
        let mut spectra = (0..self.heads)
            .map(|_| Sequence::zeros(frames, self.bins))
            .collect::<Result<Vec<_>>>()?;
        let mut harmonic = Sequence::zeros(gain.frames, gain.channels)?;
        if rows.is_empty() && !flush {
            return Ok(Some((spectra, harmonic)));
        }

        let spectrum_len = spectra[0].values.len();
        let merged_len = spectrum_len
            .checked_mul(self.heads)
            .ok_or_else(|| VoiceError::new("GPU spectral output dimensions overflow"))?;
        let mut lengths = window.lengths.clone();
        lengths.extend([
            merged_len,
            Batch::readback_len(merged_len.saturating_add(harmonic.values.len())),
        ]);
        lengths.extend(
            bands
                .iter()
                .map(|band| (band.rows.ready() - frames).saturating_mul(band.rows.width)),
        );
        let mut capacity = window.capacity.max(gain.capacity);
        let mut work = window.work.saturating_add(gain.work);
        for band in &bands {
            capacity = capacity.max(band.chain.capacity);
            work = work.saturating_add(band.chain.work);
        }
        if rows.is_empty() || self.state.pending.iter().any(Option::is_some) {
            work = work.max(MIN_AUTO_WORK);
        }
        let plan = BranchPlan {
            bands,
            gain,
            capacity,
            frames,
        };
        if !compute::can_append_gpu(&lengths) {
            return Ok(None);
        }

        let mut updates = Updates {
            convolutions: Vec::new(),
            bands: Vec::with_capacity(self.bands.len()),
        };
        let ran = compute::accelerate(None, work, MIN_AUTO_WORK, &lengths, |batch| {
            self.encode(
                batch,
                graph,
                &window,
                &plan,
                &mut updates,
                (&mut spectra, &mut harmonic),
            )
        })
        .map_err(|error| VoiceError::new(error.to_string()))?;
        if !ran {
            return Ok(None);
        }
        streams.extend(updates.convolutions);
        self.state.pending = updates.bands;
        Ok(Some((spectra, harmonic)))
    }

    fn encode(
        &self,
        batch: &mut Batch<'_>,
        graph: &ConditionalGraph<'_>,
        window: &WindowPlan<'_>,
        plan: &BranchPlan<'_>,
        updates: &mut Updates,
        (spectra, harmonic): (&mut [Sequence], &mut Sequence),
    ) -> std::result::Result<(), MatrixError> {
        let spectrum_len = spectra[0].values.len();
        let merged = batch.cleared_output(spectrum_len * self.heads);
        let scratch = [batch.output(plan.capacity), batch.output(plan.capacity)];
        let scratch = [&scratch[0], &scratch[1]];
        let validation = batch.validation_index();
        let features = if window.rows.is_empty() {
            batch.output(0)
        } else {
            graph.encode(batch, window, scratch, Some(&validation))?
        };
        for ((spec, band), previous) in self.bands.iter().zip(&plan.bands).zip(&self.state.pending)
        {
            let fresh = band.chain.encode_steps(
                batch,
                &features,
                scratch,
                &mut updates.convolutions,
                Some(&validation),
            )?;
            if band.rows.fresh != 0 {
                elementwise::validate_finite(
                    batch,
                    fresh,
                    band.rows.fresh * band.rows.width,
                    &validation,
                )?;
            }
            let previous_buffer = previous
                .as_ref()
                .map(|state| batch.resident_input(state))
                .transpose()?;
            let (from_pending, from_fresh) = band.rows.split(plan.frames);
            if from_pending != 0 {
                self.merge_band(
                    batch,
                    spec,
                    previous_buffer.as_ref().expect("queued spectral rows"),
                    &merged,
                    0..from_pending,
                    plan.frames,
                )?;
            }
            if from_fresh != 0 {
                self.merge_band(
                    batch,
                    spec,
                    fresh,
                    &merged,
                    from_pending..plan.frames,
                    plan.frames,
                )?;
            }
            updates.bands.push(band.rows.retain(
                batch,
                previous.as_ref().zip(previous_buffer.as_ref()),
                fresh,
                plan.frames,
            ));
        }
        let gain = plan.gain.encode_steps(
            batch,
            &features,
            scratch,
            &mut updates.convolutions,
            Some(&validation),
        )?;
        let mut outputs = spectra
            .iter_mut()
            .enumerate()
            .map(|(head, spectrum)| (&merged, head * spectrum_len, spectrum.values.as_mut_slice()))
            .collect::<Vec<_>>();
        outputs.push((gain, 0, &mut harmonic.values));
        batch.read_outputs(
            &mut outputs,
            Some((
                &validation,
                "in the vocoder feature network or its output heads",
            )),
        )
    }

    fn merge_band(
        &self,
        batch: &mut Batch<'_>,
        spec: &VocoderBand,
        band: &Buffer,
        merged: &Buffer,
        rows: Range<usize>,
        frames: usize,
    ) -> std::result::Result<(), MatrixError> {
        let values = rows.len() * self.heads * spec.weights.len();
        let weights = batch.weight(&spec.weights);
        let grid = batch.grid((values as u32).div_ceil(64))?;
        let mut params = [0; 16];
        params[..8].copy_from_slice(&[
            values as u32,
            grid[0],
            spec.start as u32,
            spec.weights.len() as u32,
            self.bins as u32,
            self.heads as u32,
            frames as u32,
            rows.start as u32,
        ]);
        batch.dispatch(
            Kernel {
                entry: "merge_spectral_band",
                source: include_str!("../shaders/spectral_heads.wgsl"),
            },
            grid,
            params,
            [Some(band), Some(&weights), None, Some(merged), None],
        )
    }
}
