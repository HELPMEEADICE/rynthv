use super::Runtime;
use crate::platform::web::audio;

#[derive(Default)]
pub(super) struct State {
    pub(super) seconds: f64,
    playing: bool,
    loop_range: Option<(f64, f64)>,
    metronome: bool,
}

impl Runtime {
    pub fn set_loop_range(&mut self, range: Option<(f64, f64)>) {
        if range != self.transport.loop_range {
            self.transport.loop_range = range;
            let (start, end) = range.unwrap_or((0.0, 0.0));
            audio::audio_loop(start, end);
        }
    }

    pub fn playback_clock(&self, playing: bool) -> Option<f64> {
        if !playing || !self.transport.playing {
            return None;
        }
        audio::audio_clock().or_else(|| self.preview.is_pending().then_some(self.transport.seconds))
    }

    pub fn sync_transport(&mut self, playing: bool, seconds: f64, seek: bool) {
        if seek || self.transport.playing != playing {
            audio::audio_transport(playing, seconds);
        }
        self.transport.playing = playing;
        self.transport.seconds = seconds;
    }

    pub fn audition_key(&self, note: Option<u8>) {
        audio::audition(note);
    }
    pub fn metronome_enabled(&self) -> bool {
        self.transport.metronome
    }
    pub fn set_metronome(&mut self, enabled: bool) {
        self.transport.metronome = enabled;
        self.publish_metronome();
    }

    pub(super) fn publish_metronome(&self) {
        let mut beats = Vec::new();
        if self.transport.metronome
            && let Some(project) = &self.preview.project
        {
            let time = &project.time;
            let end = self
                .planner
                .plan()
                .requests
                .values()
                .map(|request| {
                    request.coverage.sample_end.0 as f64 / f64::from(request.coverage.sample_rate)
                })
                .chain(
                    project
                        .tracks
                        .iter()
                        .filter_map(|track| track.audio_clip().map(|clip| clip.end_seconds())),
                )
                .fold(time.seconds_at(32 * crate::model::QUARTER), f64::max);
            let mut bar = 0;
            let mut tick = 0;
            while beats.len() < 64_000 && time.seconds_at(tick) <= end {
                let meter = time.meter_at(bar);
                let step = crate::model::QUARTER * 4 / i64::from(meter.denominator);
                for index in 0..meter.numerator {
                    let seconds = time.seconds_at(tick + i64::from(index) * step).max(0.0);
                    // The sign encodes accents, including an accent at -0.0.
                    beats.push(if index == 0 { -seconds } else { seconds });
                }
                tick += i64::from(meter.numerator) * step;
                bar += 1;
            }
        }
        audio::metronome(&beats);
    }
}
