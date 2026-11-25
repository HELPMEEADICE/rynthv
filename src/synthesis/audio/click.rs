pub(crate) fn click(frequency: f64, gain: f64) -> Vec<f32> {
    let frames = (f64::from(48_000u32) * 0.040) as usize;
    (0..frames)
        .map(|frame| {
            let seconds = frame as f64 / f64::from(48_000u32);
            let phase = std::f64::consts::TAU * frequency * seconds;
            let attack = (seconds / 0.0005).min(1.0);
            let tail = 1.0 - frame as f64 / frames as f64;
            let envelope = attack * (-seconds * 110.0).exp() * tail * tail;
            ((phase.sin() * 0.8 + (phase * 2.0).sin() * 0.2) * envelope * gain) as f32
        })
        .collect()
}
