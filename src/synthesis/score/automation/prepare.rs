use super::seconds_curve::SecondsCurve;
use crate::{
    model::{Automation, Parameter, VoiceParameter, VoiceSettings},
    synthesis::{
        plan::{GlobalBlick, OwnerInput},
        voice::{Result, VoiceError},
    },
};
pub(super) struct GridTimes<'a> {
    input: &'a OwnerInput,
    begin: i64,
    end: i64,
    step: i64,
    include_partial: bool,
    cached: Option<Vec<f64>>,
}

impl<'a> GridTimes<'a> {
    pub(super) fn new(
        input: &'a OwnerInput,
        begin: i64,
        end: i64,
        step: i64,
        include_partial: bool,
    ) -> Self {
        Self {
            input,
            begin,
            end,
            step,
            include_partial,
            cached: None,
        }
    }

    fn timestamps(&mut self) -> Result<&[f64]> {
        if self.cached.is_none() {
            let count = self
                .end
                .checked_sub(self.begin)
                .and_then(|span| {
                    let whole = span / self.step;
                    let count = whole + i64::from(self.include_partial && span % self.step != 0);
                    usize::try_from(count).ok()
                })
                .ok_or_else(|| VoiceError::new("Automation range overflows"))?;
            let mut times = Vec::new();
            times
                .try_reserve_exact(count)
                .map_err(|_| VoiceError::new("Automation range is too large"))?;
            let mut tempo = self.input.tempo.cursor();
            for index in 0..count {
                let local = self
                    .begin
                    .checked_add(index as i64 * self.step)
                    .ok_or_else(|| VoiceError::new("Automation grid position overflows"))?;
                let global = local
                    .checked_add(self.input.content_offset.0)
                    .ok_or_else(|| VoiceError::new("Global automation position overflows"))?;
                times.push(tempo.seconds_at(GlobalBlick(global)));
            }
            self.cached = Some(times);
        }
        Ok(self.cached.as_deref().expect("cached timestamps"))
    }
}

pub(super) struct CurveGrid {
    pub(super) default: f64,
    pub(super) baseline: f64,
    pub(super) range: (f64, f64),
    pub(super) multiply: bool,
}

pub(super) fn prepare_curve(
    grid_times: &mut GridTimes<'_>,
    curve: Option<&Automation>,
    track_curve: Option<&Automation>,
    grid: CurveGrid,
) -> Result<SecondsCurve> {
    let sampler = curve
        .map(|curve| curve.sampler(grid.default))
        .transpose()
        .map_err(VoiceError::new)?;
    let track_sampler = track_curve
        .map(|curve| curve.sampler(grid.default))
        .transpose()
        .map_err(VoiceError::new)?;
    let begin = grid_times.begin;
    let step = grid_times.step;
    let content_offset = grid_times.input.content_offset.0;
    let track_offset = grid_times.input.track_automation_offset.0;
    let times = grid_times.timestamps()?;
    let mut points = Vec::new();
    points
        .try_reserve_exact(times.len())
        .map_err(|_| VoiceError::new("Automation range is too large"))?;
    for (index, &seconds) in times.iter().enumerate() {
        let local = begin + index as i64 * step;
        let mut raw = sampler
            .as_ref()
            .map_or(Ok(grid.default), |sampler| sampler.sample(local))
            .map_err(VoiceError::new)?;
        if let Some(track) = &track_sampler {
            let global = local + content_offset;
            let track_position = global
                .checked_sub(track_offset)
                .ok_or_else(|| VoiceError::new("Track automation position overflows"))?;
            let value = track.sample(track_position).map_err(VoiceError::new)?;
            if grid.multiply {
                raw *= value;
            } else {
                raw += value;
            }
        }
        let value = ((raw + grid.baseline) as f32).clamp(grid.range.0 as f32, grid.range.1 as f32);
        points.push((seconds, value));
    }
    SecondsCurve::new(points, (grid.default + grid.baseline) as f32)
}

pub(super) fn scalar_baseline(settings: &VoiceSettings, parameter: Parameter) -> f64 {
    let parameter = match parameter {
        Parameter::Loudness => VoiceParameter::Loudness,
        Parameter::Tension => VoiceParameter::Tension,
        Parameter::Breathiness => VoiceParameter::Breathiness,
        Parameter::Gender => VoiceParameter::Gender,
        Parameter::ToneShift => VoiceParameter::ToneShift,
        _ => return 0.0,
    };
    settings
        .scalar(parameter)
        .unwrap_or_else(|| parameter.default_value())
}
