use super::super::{Result, VoiceError};

pub(super) fn decode<const WIDTH: usize>(value: &str) -> Result<[f32; WIDTH]> {
    let mut output = [0.0; WIDTH];
    if value.len() != WIDTH * 8 || !value.is_ascii() {
        return Err(VoiceError::new(format!(
            "Expected {WIDTH} little-endian Float32 values encoded as HEX"
        )));
    }
    for (target, word) in output.iter_mut().zip(value.as_bytes().as_chunks::<8>().0) {
        let mut bytes = [0; 4];
        for (byte, digits) in bytes.iter_mut().zip(word.as_chunks::<2>().0) {
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

#[cfg(test)]
mod tests {
    use super::decode;
    use crate::synthesis::voice::hex;

    #[test]
    fn synthetic_f32_hex_preserves_little_endian_bits() {
        let expected = [1.25_f32, -3.5, -0.0, f32::MIN_POSITIVE];
        let bytes = expected
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        let encoded = hex(&bytes);
        for input in [encoded.clone(), encoded.to_uppercase()] {
            assert_eq!(
                decode::<4>(&input).unwrap().map(f32::to_bits),
                expected.map(f32::to_bits)
            );
        }
    }

    #[test]
    fn synthetic_hex_rejects_corruption_and_nonfinite_values() {
        for value in [
            "",
            "0000000000",
            "000000gg",
            "0000c07f",
            "0000807f",
            "000080ff",
        ] {
            assert!(decode::<1>(value).is_err(), "{value}");
        }
    }
}
