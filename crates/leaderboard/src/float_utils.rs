use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct FloatWrapper(pub f64); // Or f32

impl Eq for FloatWrapper {}

impl Ord for FloatWrapper {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.partial_cmp(other).unwrap_or_else(|| {
			if self.0.is_nan() && other.0.is_nan() {
				std::cmp::Ordering::Equal
			} else if self.0.is_nan() {
				std::cmp::Ordering::Less
			} else {
				std::cmp::Ordering::Greater
			}
		})
	}
}

impl fmt::Display for FloatWrapper {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

// Define a trait that FloatWrapper implements
pub trait AsFloatWrapper {
	fn as_float_wrapper(self) -> FloatWrapper;
}

// Implement the trait for FloatWrapper itself (no conversion needed)
impl AsFloatWrapper for FloatWrapper {
	fn as_float_wrapper(self) -> FloatWrapper {
		self
	}
}

// Blanket implementation: For any type that is 'Copy' and can be converted 'into' f64,
// implement AsFloatWrapper by creating a FloatWrapper.
impl<T> AsFloatWrapper for T
where
	T: Copy + Into<f64>,
{
	fn as_float_wrapper(self) -> FloatWrapper {
		FloatWrapper(self.into())
	}
}
