use super::{Automation, Interpolation};

/// A validated curve with cubic coefficients prepared once for repeated queries.
pub struct AutomationSampler<'a> {
    curve: &'a Automation,
    interpolation: Interpolation,
    default: f64,
    cubic: Vec<CubicSegment>,
}

#[derive(Clone, Copy)]
struct CubicSegment {
    width: f64,
    left_tangent: f64,
    right_tangent: f64,
}

impl<'a> AutomationSampler<'a> {
    pub(super) fn new(curve: &'a Automation, default: f64) -> Result<Self, String> {
        curve.validate()?;
        if !(default as f32).is_finite() {
            return Err("参数默认值必须为有限数".into());
        }
        let interpolation = curve.interpolation()?;
        let cubic = if interpolation == Interpolation::Cubic && curve.points.len() >= 3 {
            (0..curve.points.len() - 1)
                .map(|index| CubicSegment::new(&curve.points, index))
                .collect()
        } else {
            Vec::new()
        };
        Ok(Self {
            curve,
            interpolation,
            default: f64::from(default as f32),
            cubic,
        })
    }

    pub fn sample(&self, position: i64) -> Result<f64, String> {
        let points = &self.curve.points;
        let Some(first) = points.first() else {
            return Ok(self.default);
        };
        if position <= first.0 {
            return Ok(f64::from(first.1 as f32));
        }
        let last = points.last().unwrap();
        if position >= last.0 {
            return Ok(f64::from(last.1 as f32));
        }
        let right = points.partition_point(|point| point.0 < position);
        let index = right - 1;
        let left_point = points[index];
        let right_point = points[right];
        if position == right_point.0 {
            return Ok(f64::from(right_point.1 as f32));
        }
        let elapsed = (i128::from(position) - i128::from(left_point.0)) as f64;
        let width = (right_point.0 as i128 - left_point.0 as i128) as f64;
        let fraction = elapsed / width;
        let left_value = left_point.1 as f32;
        let right_value = right_point.1 as f32;
        let value = match self.interpolation {
            Interpolation::Cosine => {
                let fraction = (1.0 - (fraction * std::f64::consts::PI).cos()) * 0.5;
                linear(left_value, right_value, fraction)
            }
            Interpolation::Cubic if !self.cubic.is_empty() => {
                self.cubic[index].sample(left_value, right_value, elapsed)
            }
            Interpolation::Linear | Interpolation::Cubic => {
                linear(left_value, right_value, fraction)
            }
        };
        if !value.is_finite() {
            return Err("参数曲线求值超出有限数范围".into());
        }
        Ok(f64::from(value))
    }
}

impl CubicSegment {
    fn new(points: &[(i64, f64)], index: usize) -> Self {
        let (left_time, left_value) = points[index];
        let (right_time, right_value) = points[index + 1];
        let left_time = i128::from(left_time);
        let right_time = i128::from(right_time);
        let (previous_time, previous_value) = if index == 0 {
            (left_time - 1, left_value)
        } else {
            let point = points[index - 1];
            (i128::from(point.0), point.1)
        };
        let (next_time, next_value) = if index + 2 == points.len() {
            (right_time + 1, right_value)
        } else {
            let point = points[index + 2];
            (i128::from(point.0), point.1)
        };
        let left_value = f64::from(left_value as f32);
        let right_value = f64::from(right_value as f32);
        let previous_value = f64::from(previous_value as f32);
        let next_value = f64::from(next_value as f32);
        let width = (right_time - left_time) as f64;
        let distance = (width / 705_600_000.0 - 0.25).max(0.0);
        let weight = if distance <= 4.0 {
            1.0 / (distance * 10.0 + 1.0)
        } else {
            0.0
        };
        // Tangents are scaled to the unit interval [0, 1], so the linear fallback
        // term is dy/du = (right_value - left_value) without dividing by width.
        let linear_tangent = (right_value - left_value) * (1.0 - weight);
        let left_slope = (right_value - previous_value) / (right_time - previous_time) as f64;
        let right_slope = (next_value - left_value) / (next_time - left_time) as f64;
        Self {
            width,
            left_tangent: left_slope * width * weight + linear_tangent,
            right_tangent: right_slope * width * weight + linear_tangent,
        }
    }

    fn sample(self, left: f32, right: f32, elapsed: f64) -> f32 {
        let fraction = elapsed / self.width;
        let squared = fraction * fraction;
        let cubed = squared * fraction;
        let left_basis = 2.0 * cubed - 3.0 * squared + 1.0;
        let left_tangent_basis = cubed - 2.0 * squared + fraction;
        let right_basis = 3.0 * squared - 2.0 * cubed;
        let right_tangent_basis = cubed - squared;
        let value = left_tangent_basis * self.left_tangent + left_basis * f64::from(left);
        let value = value + right_basis * f64::from(right);
        (value + right_tangent_basis * self.right_tangent) as f32
    }
}

fn linear(left: f32, right: f32, fraction: f64) -> f32 {
    (f64::from(left) * (1.0 - fraction) + f64::from(right) * fraction) as f32
}
