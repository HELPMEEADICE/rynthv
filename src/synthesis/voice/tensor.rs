use std::sync::Arc;

use serde::Serialize;

use super::{Result, VoiceError, array, product, sum, u32_at};

/// File storage metadata. The inference layer separately defines execution layout.
#[derive(Clone, Debug, Serialize)]
pub struct TensorLayout {
    pub storage: &'static str,
    pub rows: usize,
    pub columns: usize,
    pub elements: usize,
    pub scales: usize,
    pub stored_bits: u32,
}

/// Owned immutable tensor values. Only `TensorLayout`, never weights, is serialized.
/// Quantized bytes remain in file order; execution applies the decoding formula.
#[derive(Clone)]
pub enum TensorStorage {
    FloatMatrix {
        rows: usize,
        columns: usize,
        values: Arc<[f32]>,
    },
    FloatVector {
        values: Arc<[f32]>,
    },
    QuantizedMatrix {
        rows: usize,
        columns: usize,
        bits: u32,
        input_exponent: i32,
        scales: Arc<[f32]>,
        values: Arc<[u8]>,
    },
}

impl TensorStorage {
    pub fn dense(&self) -> Result<crate::synthesis::inference::matrix::DenseMatrix> {
        use crate::synthesis::inference::matrix::DenseMatrix;
        let result = match self {
            Self::FloatMatrix {
                rows,
                columns,
                values,
            } => DenseMatrix::from_float(*rows, *columns, Arc::clone(values)),
            Self::QuantizedMatrix {
                rows,
                columns,
                bits,
                input_exponent,
                scales,
                values,
            } => {
                DenseMatrix::from_quantized(*rows, *columns, *bits, *input_exponent, scales, values)
            }
            Self::FloatVector { .. } => return Err(VoiceError::new("Expected matrix storage")),
        };
        result.map_err(|error| VoiceError::new(error.to_string()))
    }
    pub fn layout(&self) -> TensorLayout {
        match self {
            Self::FloatMatrix {
                rows,
                columns,
                values,
            } => TensorLayout {
                storage: "float32_matrix",
                rows: *rows,
                columns: *columns,
                elements: values.len(),
                scales: 0,
                stored_bits: 32,
            },
            Self::FloatVector { values } => TensorLayout {
                storage: "float32_vector",
                rows: 1,
                columns: values.len(),
                elements: values.len(),
                scales: 0,
                stored_bits: 32,
            },
            Self::QuantizedMatrix {
                rows,
                columns,
                bits,
                scales,
                ..
            } => TensorLayout {
                storage: "integer_matrix_with_row_scales",
                rows: *rows,
                columns: *columns,
                elements: rows.saturating_mul(*columns),
                scales: scales.len(),
                stored_bits: *bits,
            },
        }
    }

    pub fn vector(&self) -> Option<&[f32]> {
        match self {
            Self::FloatVector { values } => Some(values),
            _ => None,
        }
    }

    pub fn matrix_shape(&self) -> Option<(usize, usize)> {
        match self {
            Self::FloatMatrix { rows, columns, .. }
            | Self::QuantizedMatrix { rows, columns, .. } => Some((*rows, *columns)),
            Self::FloatVector { .. } => None,
        }
    }
}

/// Parse once into typed storage after the exact-size and overflow checks.
pub(super) fn load(type_id: [u8; 16], payload: &[u8]) -> Result<TensorStorage> {
    let layout = parse(type_id, payload, payload.len())?;
    match type_id {
        FLOAT_MATRIX => Ok(TensorStorage::FloatMatrix {
            rows: layout.rows,
            columns: layout.columns,
            values: floats(&payload[8..]),
        }),
        FLOAT_VECTOR => Ok(TensorStorage::FloatVector {
            values: floats(&payload[4..]),
        }),
        QUANTIZED => {
            let matrix = sum(16, product(layout.scales, 4)?)?;
            Ok(TensorStorage::QuantizedMatrix {
                rows: layout.rows,
                columns: layout.columns,
                bits: layout.stored_bits,
                input_exponent: u32_at(payload, 4)? as i32,
                scales: floats(&payload[16..matrix]),
                values: Arc::from(&payload[matrix + 8..]),
            })
        }
        _ => unreachable!("validated tensor identity"),
    }
}

fn floats(bytes: &[u8]) -> Arc<[f32]> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect()
}

// Tensor type identities in DNNI v3 resources.
const FLOAT_MATRIX: [u8; 16] = [
    0x16, 0xff, 0x49, 0x6e, 0xfa, 0x84, 0x6a, 0x4f, 0xd8, 0xf6, 0xcb, 0xce, 0x85, 0x85, 0x6b, 0x3d,
];
const FLOAT_VECTOR: [u8; 16] = [
    0x19, 0xd7, 0x8b, 0x4c, 0x9d, 0x82, 0x95, 0xbe, 0x5d, 0x83, 0x3b, 0x1e, 0x99, 0x97, 0x82, 0x44,
];
const QUANTIZED: [u8; 16] = [
    0xce, 0x8d, 0x62, 0x7e, 0x89, 0x87, 0xe6, 0x38, 0xf9, 0x4d, 0xeb, 0xca, 0xa7, 0x70, 0x46, 0xdc,
];

pub(super) fn is_known(type_id: [u8; 16]) -> bool {
    [FLOAT_MATRIX, FLOAT_VECTOR, QUANTIZED].contains(&type_id)
}

pub(super) fn required_inspection_bytes(prefix: &[u8], total_len: usize) -> Result<usize> {
    match array(prefix, 0)? {
        FLOAT_MATRIX => Ok(24),
        FLOAT_VECTOR => Ok(20),
        QUANTIZED => {
            let stored_bits = u32_at(prefix, 16)?;
            if ![8, 16].contains(&stored_bits) || u32_at(prefix, 24)? != 0 {
                return Err(VoiceError::new("Invalid observed integer matrix header"));
            }
            let scales = u32_at(prefix, 28)? as usize;
            let matrix = sum(16, product(scales, 4)?)?;
            let header = sum(16, sum(matrix, 8)?)?;
            if header > total_len {
                return Err(VoiceError::new(
                    "Tensor storage does not exactly consume its resource",
                ));
            }
            Ok(header)
        }
        _ => Err(VoiceError::new("Unknown tensor storage identity")),
    }
}

pub(super) fn parse(
    type_id: [u8; 16],
    payload: &[u8],
    payload_total: usize,
) -> Result<TensorLayout> {
    if payload_total < payload.len() {
        return Err(VoiceError::new(
            "Tensor storage does not exactly consume its resource",
        ));
    }
    match type_id {
        FLOAT_MATRIX => {
            let rows = u32_at(payload, 0)? as usize;
            let columns = u32_at(payload, 4)? as usize;
            let elements = product(rows, columns)?;
            exact_length(payload_total, sum(8, product(elements, 4)?)?)?;
            Ok(TensorLayout {
                storage: "float32_matrix",
                rows,
                columns,
                elements,
                scales: 0,
                stored_bits: 32,
            })
        }
        FLOAT_VECTOR => {
            let elements = u32_at(payload, 0)? as usize;
            exact_length(payload_total, sum(4, product(elements, 4)?)?)?;
            Ok(TensorLayout {
                storage: "float32_vector",
                rows: 1,
                columns: elements,
                elements,
                scales: 0,
                stored_bits: 32,
            })
        }
        QUANTIZED => {
            let stored_bits = u32_at(payload, 0)?;
            if ![8, 16].contains(&stored_bits) || u32_at(payload, 8)? != 0 {
                return Err(VoiceError::new("Invalid observed integer matrix header"));
            }
            let scales = u32_at(payload, 12)? as usize;
            let matrix = sum(16, product(scales, 4)?)?;
            let rows = u32_at(payload, matrix)? as usize;
            let columns = u32_at(payload, sum(matrix, 4)?)? as usize;
            if rows != scales {
                return Err(VoiceError::new("Integer matrix row/scale count mismatch"));
            }
            let elements = product(rows, columns)?;
            exact_length(
                payload_total,
                sum(
                    sum(matrix, 8)?,
                    product(elements, stored_bits as usize / 8)?,
                )?,
            )?;
            for offset in (16..matrix).step_by(4) {
                let scale = f32::from_le_bytes(array(payload, offset)?);
                if !scale.is_finite() || scale < 0.0 {
                    return Err(VoiceError::new("Integer matrix contains an invalid scale"));
                }
            }
            Ok(TensorLayout {
                storage: "integer_matrix_with_row_scales",
                rows,
                columns,
                elements,
                scales,
                stored_bits,
            })
        }
        _ => Err(VoiceError::new("Unknown tensor storage identity")),
    }
}

fn exact_length(actual: usize, expected: usize) -> Result<()> {
    if actual != expected {
        return Err(VoiceError::new(
            "Tensor storage does not exactly consume its resource",
        ));
    }
    Ok(())
}
