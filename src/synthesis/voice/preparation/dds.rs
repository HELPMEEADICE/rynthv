use super::{PreparedVoice, Result, VoiceError};
use crate::synthesis::stages::{
    sv1_duration::Sv1DurationStage, sv1_f0::Sv1F0Stage, sv1_timbre::Sv1TimbreNetwork,
};

impl PreparedVoice {
    pub(crate) fn native_duration(&self) -> Result<Sv1DurationStage<'_>> {
        let timing = self.flat.as_ref().and_then(|flat| flat.timing.as_ref());
        Sv1DurationStage::prepare_conditioned(
            self.model("duration")?,
            timing.map(|row| row.as_slice()),
        )
    }

    pub(crate) fn native_pitch(&self) -> Result<Sv1F0Stage<'_>> {
        let enhancement = self.f0_enhancement.ok_or_else(|| {
            VoiceError::new("SV1 voice is missing its native F0 enhancement configuration")
        })?;
        let pitch = self.flat.as_ref().and_then(|flat| flat.pitch.as_ref());
        Sv1F0Stage::prepare_native_conditioned(
            self.model("f0")?,
            enhancement,
            pitch.map(|row| row.as_slice()),
        )
    }

    pub(crate) fn native_timbre(&self) -> Result<Sv1TimbreNetwork<'_>> {
        Sv1TimbreNetwork::prepare_native_conditioned(self.model("timbre")?, self.flat.as_ref())
    }
}
