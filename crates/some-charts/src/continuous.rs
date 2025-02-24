use std::cmp::{max, min};
use std::f64::NAN;

// Constants
static UNIT: [f64; 2] = [0.0, 1.0];

// Identity function
fn identity(x: f64) -> f64 {
	x
}

// Normalize function
fn normalize(a: f64, b: f64) -> Box<dyn Fn(f64) -> f64> {
	let diff = b - a;
	if diff != 0.0 {
		Box::new(move |x| (x - a) / diff)
	} else {
		Box::new(move |_| if diff.is_nan() { NAN } else { 0.5 })
	}
}

// Clamper function
fn clamper(a: f64, b: f64) -> Box<dyn Fn(f64) -> f64> {
	let (a, b) = if a > b { (b, a) } else { (a, b) };
	Box::new(move |x| max(a, min(b, x)))
}

// Bimap function
fn bimap(domain: &[f64], range: &[f64], interpolate: &dyn Fn(f64, f64) -> Box<dyn Fn(f64) -> f64>) -> Box<dyn Fn(f64) -> f64> {
	let (d0, d1, r0, r1) = (domain[0], domain[1], range[0], range[1]);
	let (d0_norm, r0_interp) = if d1 < d0 {
		(normalize(d1, d0), interpolate(r1, r0))
	} else {
		(normalize(d0, d1), interpolate(r0, r1))
	};
	let d0_norm = d0_norm;
	let r0_interp = r0_interp;
	Box::new(move |x| r0_interp(d0_norm(x)))
}

// Bisect function implementation
fn bisect(domain: &[f64], x: f64, lo: usize, hi: usize) -> usize {
	if x < domain[lo] {
		return lo;
	}
	if x > domain[hi - 1] {
		return hi;
	}
	let mut lo = lo;
	let mut hi = hi;
	while lo < hi {
		let mid = (lo + hi) / 2;
		if x >= domain[mid] {
			lo = mid + 1;
		} else {
			hi = mid;
		}
	}
	lo
}

// Polymap function
fn polymap(domain: &[f64], range: &[f64], interpolate: &dyn Fn(f64, f64) -> Box<dyn Fn(f64) -> f64>) -> Box<dyn Fn(f64) -> f64> {
	let j = min(domain.len(), range.len()) - 1;
	let mut d = Vec::with_capacity(j);
	let mut r = Vec::with_capacity(j);

	// Reverse descending domains
	let (domain, range) = if domain[j] < domain[0] {
		(domain.iter().rev().cloned().collect::<Vec<_>>(), range.iter().rev().cloned().collect::<Vec<_>>())
	} else {
		(domain.to_vec(), range.to_vec())
	};

	for i in 0..j {
		d.push(normalize(domain[i], domain[i + 1]));
		r.push(interpolate(range[i], range[i + 1]));
	}

	let d = d;
	let r = r;
	let domain = domain;

	Box::new(move |x| {
		let i = bisect(&domain, x, 0, j + 1) - 1;
		if i >= r.len() {
			return range[range.len() - 1];
		}
		r[i](d[i](x))
	})
}

// Transformer struct
#[derive(Clone)]
struct Transformer {
	domain: Vec<f64>,
	range: Vec<f64>,
	interpolate: Box<dyn Fn(f64, f64) -> Box<dyn Fn(f64) -> f64>>,
	transform: Box<dyn Fn(f64) -> f64>,
	untransform: Box<dyn Fn(f64) -> f64>,
	unknown: Option<f64>,
	clamp: Box<dyn Fn(f64) -> f64>,
	piecewise: Option<Box<dyn Fn(f64) -> f64>>,
	output: Option<Box<dyn Fn(f64) -> f64>>,
	input: Option<Box<dyn Fn(f64) -> f64>>,
}

impl Transformer {
	fn new() -> Self {
		Self {
			domain: UNIT.to_vec(),
			range: UNIT.to_vec(),
			interpolate: Box::new(|a, b| Box::new(move |t| a + (b - a) * t)),
			transform: Box::new(identity),
			untransform: Box::new(identity),
			unknown: None,
			clamp: Box::new(identity),
			piecewise: None,
			output: None,
			input: None,
		}
	}

	fn rescale(&mut self) -> &mut Self {
		let n = min(self.domain.len(), self.range.len());
		if n < 2 {
			return self;
		}

		if self.clamp(0.0) != identity(0.0) {
			self.clamp = clamper(self.domain[0], self.domain[n - 1]);
		}

		self.piecewise = Some(if n > 2 {
			polymap(&self.domain, &self.range, &*self.interpolate)
		} else {
			bimap(&self.domain, &self.range, &*self.interpolate)
		});

		self.output = None;
		self.input = None;
		self
	}

	fn scale(&mut self, x: f64) -> f64 {
		if x.is_nan() {
			return self.unknown.unwrap_or(NAN);
		}

		if self.piecewise.is_none() {
			self.rescale();
		}

		let transformed = (self.transform)((self.clamp)(x));
		match &self.piecewise {
			Some(piecewise) => piecewise(transformed),
			None => transformed,
		}
	}

	fn invert(&mut self, y: f64) -> f64 {
		if self.piecewise.is_none() {
			self.rescale();
		}

		let piecewise = self.piecewise.as_ref().unwrap();
		(self.clamp)((self.untransform)(piecewise(y)))
	}

	fn domain(&mut self, domain: Vec<f64>) -> &mut Self {
		self.domain = domain;
		self.rescale()
	}

	fn range(&mut self, range: Vec<f64>) -> &mut Self {
		self.range = range;
		self.rescale()
	}

	fn range_round(&mut self, range: Vec<f64>) -> &mut Self {
		self.range = range;
		self.interpolate = Box::new(|a, b| Box::new(move |t| (a + (b - a) * t).round()));
		self.rescale()
	}

	fn clamp(&mut self, clamp: bool) -> &mut Self {
		self.clamp = if clamp {
			clamper(self.domain[0], self.domain[self.domain.len() - 1])
		} else {
			Box::new(identity)
		};
		self.rescale()
	}

	fn interpolate(&mut self, interpolate: Box<dyn Fn(f64, f64) -> Box<dyn Fn(f64) -> f64>>) -> &mut Self {
		self.interpolate = interpolate;
		self.rescale()
	}

	fn unknown(&mut self, unknown: Option<f64>) -> &mut Self {
		self.unknown = unknown;
		self
	}

	fn set_transform(&mut self, transform: Box<dyn Fn(f64) -> f64>, untransform: Box<dyn Fn(f64) -> f64>) -> &mut Self {
		self.transform = transform;
		self.untransform = untransform;
		self.rescale()
	}
}

// Continuous function
fn continuous() -> Transformer {
	Transformer::new()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_identity() {
		assert_eq!(identity(5.0), 5.0);
		assert_eq!(identity(-3.0), -3.0);
	}

	#[test]
	fn test_normalize() {
		let norm = normalize(0.0, 10.0);
		assert_eq!(norm(5.0), 0.5);
		assert_eq!(norm(10.0), 1.0);
		assert_eq!(norm(0.0), 0.0);
		assert_eq!(norm(15.0), 1.5); // Out of bounds
	}

	#[test]
	fn test_clamper() {
		let clamp = clamper(0.0, 10.0);
		assert_eq!(clamp(5.0), 5.0);
		assert_eq!(clamp(-5.0), 0.0); // Clamped to lower bound
		assert_eq!(clamp(15.0), 10.0); // Clamped to upper bound
	}

	#[test]
	fn test_bimap() {
		let bimap_fn = bimap(&[0.0, 10.0], &[0.0, 100.0], &|a, b| Box::new(move |t| a + (b - a) * t));
		assert_eq!(bimap_fn(5.0), 50.0);
		assert_eq!(bimap_fn(0.0), 0.0);
		assert_eq!(bimap_fn(10.0), 100.0);
	}

	#[test]
	fn test_polymap() {
		let polymap_fn = polymap(&[0.0, 5.0, 10.0], &[0.0, 50.0, 100.0], &|a, b| Box::new(move |t| a + (b - a) * t));
		assert_eq!(polymap_fn(2.5), 25.0);
		assert_eq!(polymap_fn(7.5), 75.0);
		assert_eq!(polymap_fn(0.0), 0.0);
		assert_eq!(polymap_fn(10.0), 100.0);
	}

	#[test]
	fn test_transformer_scale() {
		let mut scale = continuous();
		scale.domain(vec![0.0, 10.0]).range(vec![0.0, 100.0]);
		assert_eq!(scale.scale(5.0), 50.0);
		assert_eq!(scale.scale(0.0), 0.0);
		assert_eq!(scale.scale(10.0), 100.0);
	}

	#[test]
	fn test_transformer_invert() {
		let mut scale = continuous();
		scale.domain(vec![0.0, 10.0]).range(vec![0.0, 100.0]);
		assert_eq!(scale.invert(50.0), 5.0);
		assert_eq!(scale.invert(0.0), 0.0);
		assert_eq!(scale.invert(100.0), 10.0);
	}

	#[test]
	fn test_transformer_clamp() {
		let mut scale = continuous();
		scale.domain(vec![0.0, 10.0]).range(vec![0.0, 100.0]).clamp(true);
		assert_eq!(scale.scale(-5.0), 0.0); // Clamped to lower bound
		assert_eq!(scale.scale(15.0), 100.0); // Clamped to upper bound
	}

	#[test]
	fn test_transformer_range_round() {
		let mut scale = continuous();
		scale.domain(vec![0.0, 10.0]).range_round(vec![0.0, 100.0]);
		assert_eq!(scale.scale(5.5), 55.0); // Rounded to nearest integer
	}

	#[test]
	fn test_transformer_unknown() {
		let mut scale = continuous();
		scale.domain(vec![0.0, 10.0]).range(vec![0.0, 100.0]).unknown(NAN);
		assert!(scale.scale(f64::NAN).is_nan()); // Returns NaN for unknown values
	}

	#[test]
	fn test_transformer_set_transform() {
		let mut scale = continuous();
		scale.domain(vec![0.0, 10.0]).range(vec![0.0, 100.0]).set_transform(
			Box::new(|x| x * 2.0), // Transform: double the input
			Box::new(|y| y / 2.0), // Untransform: halve the output
		);
		assert_eq!(scale.scale(5.0), 50.0); // Transformed input: 10.0 -> 100.0, then scaled
		assert_eq!(scale.invert(50.0), 5.0); // Untransformed output: 25.0 -> 5.0
	}
}
