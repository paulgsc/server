/// This is a fork of <https://github.com/d3/d3-array/blob/d6c195ab0f21b5fe30cd2a32612410998d281ecc/src/ticks.js>.
///
/// We(Me, Myself and I) intentionally forked this because:
/// - Just a noob doing nooby stuff!? 😅
///
/// # License
/// This project is licensed under the terms specified in  
/// <https://github.com/d3/d3-array/blob/d6c195ab0f21b5fe30cd2a32612410998d281ecc/LICENSE>.

const E10: f64 = 7.071_067_811_865_475_5; // sqrt(50.0)
const E5: f64 = 3.162_277_660_168_379_5; // sqrt(10.0)
const E2: f64 = 1.414_213_562_373_095_1; // sqrt(2.0)

fn tick_spec(start: f64, stop: f64, count: f64) -> Option<(i64, i64, f64)> {
	if count <= 0.0 {
		return None;
	}

	let tick_step = (stop - start) / count.max(0.0);
	let power = tick_step.log10().floor() as i32;
	let error = tick_step / 10f64.powi(power);

	let factor = match error {
		e if e >= E10 => 10.0,
		e if e >= E5 => 5.0,
		e if e >= E2 => 2.0,
		_ => 1.0,
	};

	let (i1, i2, inc) = if power < 0 {
		let inc = 10f64.powi(-power) / factor;
		let i1 = (start * inc).round() as i64;
		let i2 = (stop * inc).round() as i64;
		let i1 = if (i1 as f64) / inc < start { i1 + 1 } else { i1 };
		let i2 = if (i2 as f64) / inc > stop { i2 - 1 } else { i2 };
		(i1, i2, -inc)
	} else {
		let inc = 10f64.powi(power) * factor;
		let i1 = (start / inc).round() as i64;
		let i2 = (stop / inc).round() as i64;
		let i1 = if (i1 as f64) * inc < start { i1 + 1 } else { i1 };
		let i2 = if (i2 as f64) * inc > stop { i2 - 1 } else { i2 };
		(i1, i2, inc)
	};

	if i2 < i1 && (0.5..2.0).contains(&count) {
		return tick_spec(start, stop, count * 2.0);
	}

	Some((i1, i2, inc))
}

pub fn ticks(start: f64, stop: f64, count: f64) -> Vec<f64> {
	if count <= 0.0 {
		return vec![];
	}
	if start == stop {
		return vec![start];
	}

	let reverse = stop < start;
	let (start, stop) = if reverse { (stop, start) } else { (start, stop) };

	tick_spec(start, stop, count).map_or_else(Vec::new, |(i1, i2, inc)| {
		if i2 < i1 {
			return vec![];
		}

		let n = (i2 - i1 + 1) as usize;
		let result: Vec<f64> = match reverse {
			true => (0..n)
				.map(|i| {
					let value = (i2 - i as i64) as f64;
					if inc < 0.0 {
						value / -inc
					} else {
						value * inc
					}
				})
				.collect(),
			false => (0..n)
				.map(|i| {
					let value = (i1 + i as i64) as f64;
					if inc < 0.0 {
						value / -inc
					} else {
						value * inc
					}
				})
				.collect(),
		};
		result
	})
}

pub fn tick_increment(start: f64, stop: f64, count: f64) -> Option<f64> {
	tick_spec(start, stop, count).map(|(_, _, inc)| inc)
}

pub fn tick_step(start: f64, stop: f64, count: f64) -> Option<f64> {
	let reverse = stop < start;
	tick_increment(if reverse { stop } else { start }, if reverse { start } else { stop }, count).map(|inc| {
		let sign = if reverse { -1.0 } else { 1.0 };
		sign * if inc < 0.0 { 1.0 / -inc } else { inc }
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_tick_spec() {
		assert_eq!(tick_spec(0.0, 10.0, 5.0), Some((0, 5, 2.0)));
		assert_eq!(tick_spec(10.0, 0.0, 5.0), Some((0, 5, -2.0)));
		assert_eq!(tick_spec(0.0, 0.0, 5.0), Some((0, 0, 1.0)));
	}

	#[test]
	fn test_ticks() {
		assert_eq!(ticks(0.0, 10.0, 5.0), vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
		assert_eq!(ticks(10.0, 0.0, 5.0), vec![10.0, 8.0, 6.0, 4.0, 2.0, 0.0]);
		assert_eq!(ticks(0.0, 0.0, 5.0), vec![0.0]);
	}

	#[test]
	fn test_tick_increment() {
		assert_eq!(tick_increment(0.0, 10.0, 5.0), Some(2.0));
	}

	#[test]
	fn test_tick_step() {
		assert_eq!(tick_step(0.0, 10.0, 5.0), Some(2.0));
		assert_eq!(tick_step(10.0, 0.0, 5.0), Some(-2.0));
	}
}
