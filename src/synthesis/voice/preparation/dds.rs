use super::{PreparedVoice, Result, VoiceError};
use crate::synthesis::stages::{
    sv1_duration::Sv1DurationStage, sv1_f0::Sv1F0Stage, sv1_timbre::Sv1TimbreNetwork,
};
use crate::synthesis::voice::{PhoneResolver, PhoneSet};

impl PreparedVoice {
    pub fn phone_inventory(&self) -> Result<Vec<(&str, &[String])>> {
        Ok(self.duration_phones()?.inventory())
    }

    /// Interpolate Flat timbre conditions in the same validated acoustic
    /// model. Timing, pitch, phonemes and selectable modes belong to self.
    pub fn blend_flat(&self, other: &Self, weight: f32) -> Result<Self> {
        if !(0.0..=1.0).contains(&weight) {
            return Err(VoiceError::new("Flat blend weight must be between 0 and 1"));
        }
        let (Some(primary), Some(secondary)) = (&self.flat, &other.flat) else {
            return Err(VoiceError::new("Flat blending requires two Flat voices"));
        };
        if !std::ptr::eq(self.model("timbre")?, other.model("timbre")?) {
            return Err(VoiceError::new(
                "Flat blending requires identical acoustic model weights",
            ));
        }
        Ok(Self::new(
            self.files.clone(),
            self.family,
            self.f0_enhancement,
            self.style_names.clone(),
            Some(primary.blend_timbre(secondary, weight)),
        ))
    }

    pub(crate) fn phones(&self) -> &[PhoneSet] {
        self.flat
            .as_ref()
            .map_or(&[], |flat| flat.phones.as_slice())
    }

    pub(crate) fn duration_phones(&self) -> Result<PhoneResolver<'_>> {
        self.model("duration")?.phone_resolver(self.phones())
    }

    pub(crate) fn native_duration(&self) -> Result<Sv1DurationStage<'_>> {
        let timing = self.flat.as_ref().and_then(|flat| flat.timing.as_ref());
        Sv1DurationStage::prepare_conditioned(
            self.model("duration")?,
            timing.map(|row| row.as_slice()),
            self.phones(),
        )
    }

    pub(crate) fn native_pitch(&self) -> Result<Sv1F0Stage<'_>> {
        let enhancement = self.f0_enhancement.ok_or_else(|| {
            VoiceError::new("SV1 voice is missing its native F0 enhancement configuration")
        })?;
        Sv1F0Stage::prepare_native_conditioned(self.model("f0")?, enhancement, self.flat.as_ref())
    }

    pub(crate) fn native_timbre(&self) -> Result<Sv1TimbreNetwork<'_>> {
        Sv1TimbreNetwork::prepare_native_conditioned(self.model("timbre")?, self.flat.as_ref())
    }
}
