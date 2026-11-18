use super::super::{Result, VoiceError};

pub(super) fn decode<const WIDTH: usize>(value: &str) -> Result<[f32; WIDTH]> {
    let mut output = [0.0; WIDTH];
    if value.len() != WIDTH * 8 || !value.is_ascii() {
        return Err(VoiceError::new(format!(
            "Expected {WIDTH} little-endian Float32 values encoded as HEX"
        )));
    }
    for (target, word) in output.iter_mut().zip(value.as_bytes().chunks_exact(8)) {
        let mut bytes = [0; 4];
        for (byte, digits) in bytes.iter_mut().zip(word.chunks_exact(2)) {
            let hex = |digit: u8| match digit {
                b'0'..=b'9' => Ok(digit - b'0'),
                b'a'..=b'f' => Ok(digit - b'a' + 10),
                b'A'..=b'F' => Ok(digit - b'A' + 10),
                _ => Err(VoiceError::new("Invalid HEX digit in Flat embedding")),
            };
            *byte = hex(digits[0])? << 4 | hex(digits[1])?;
        }
        *target = f32::from_le_bytes(bytes);
        if !target.is_finite() {
            return Err(VoiceError::new("Non-finite Float32 in Flat embedding"));
        }
    }
    Ok(output)
}
