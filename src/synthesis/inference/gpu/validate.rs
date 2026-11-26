use super::{Device, batch::Kernel};
use crate::synthesis::inference::compute::Activity;

pub(in super::super) fn validate(device: &Device) -> Result<Vec<&'static str>, String> {
    let mut cache = device.cache.lock().unwrap();
    let mut activity = Activity::default();
    let mut compiled = Vec::new();
    for (source, entries) in [
        (include_str!("shaders/affine.wgsl"), &["affine"][..]),
        (
            include_str!("shaders/spectral_heads.wgsl"),
            &["merge_spectral_band"],
        ),
        (
            include_str!("shaders/convolution.wgsl"),
            &["convolution", "transposed_convolution"],
        ),
        (
            include_str!("shaders/gru.wgsl"),
            &["gru_input", "gru_recurrent"],
        ),
        (
            include_str!("shaders/attention.wgsl"),
            &["attention_scores", "attention_softmax", "attention_values"],
        ),
        (
            include_str!("shaders/elementwise.wgsl"),
            &[
                "activation",
                "channel_affine",
                "layer_norm",
                "gated_activation",
                "embedding",
                "validate_finite",
            ],
        ),
        (
            include_str!("shaders/conditional.wgsl"),
            &["conditional_sum", "conditional_sum_projected"],
        ),
    ] {
        for &entry in entries {
            cache.pipeline(device, Kernel { source, entry }, &mut activity);
            if let Some(error) = device.error() {
                return Err(error);
            }
            compiled.push(entry);
        }
    }
    Ok(compiled)
}
