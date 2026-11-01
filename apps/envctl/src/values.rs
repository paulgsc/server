//! Whether a binary would take a value at start: its kind, then its bounds
//! while their conditions hold (`app_config::schema::unmet`, #580).

use crate::schema::{Bounds, Condition, Kind, Variable, SET};
use std::net::SocketAddr;

/// `min`/`max` as the binary says them: whole numbers without a `.0`.
pub(crate) fn number(value: f64) -> String {
	if value.fract() == 0.0 && value.abs() < 1e15 {
		#[allow(clippy::cast_possible_truncation)] // A whole number below 1e15 fits.
		let whole = value as i64;
		whole.to_string()
	} else {
		value.to_string()
	}
}

/// "at least 60", "at most 1", or "from 0 to 1".
pub(crate) fn bound_text(bounds: &Bounds) -> String {
	match (bounds.min, bounds.max) {
		(Some(low), Some(high)) => format!("from {} to {}", number(low), number(high)),
		(Some(low), None) => format!("at least {}", number(low)),
		(None, Some(high)) => format!("at most {}", number(high)),
		(None, None) => String::new(),
	}
}

/// Whether `when` holds, `current` giving each variable's value (its default
/// included).
pub(crate) fn holds(when: &Condition, current: &dyn Fn(&str) -> Option<String>) -> bool {
	let value = current(&when.variable);
	if when.value == SET {
		value.is_some_and(|value| !value.trim().is_empty())
	} else {
		value.as_deref() == Some(when.value.as_str())
	}
}

/// Why `variable`'s bounds refuse `value` while their conditions hold, or
/// None. A value that is not a number is the kind check's to refuse.
pub(crate) fn bound_refusal(variable: &Variable, value: &str, current: &dyn Fn(&str) -> Option<String>) -> Option<String> {
	let bounds = variable.bounds.as_ref()?;
	if !bounds.when.iter().all(|when| holds(when, current)) {
		return None;
	}
	let number: f64 = value.trim().parse().ok()?;
	let below = bounds.min.is_some_and(|low| number < low);
	let above = bounds.max.is_some_and(|high| number > high);
	(below || above).then(|| format!("must be {}", bound_text(bounds)))
}

/// Why `variable`'s binary would refuse `value` at start, or None.
pub(crate) fn refusal(variable: &Variable, value: &str) -> Option<String> {
	let items: Vec<&str> = variable.delimiter.map_or_else(|| vec![value], |delimiter| value.split(delimiter).collect());
	items.into_iter().find_map(|item| item_refusal(variable, item))
}

fn item_refusal(variable: &Variable, item: &str) -> Option<String> {
	let whole = |limit: u128| {
		let digits = item.strip_prefix('+').unwrap_or(item);
		let fits = !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) && digits.parse::<u128>().is_ok_and(|value| value < limit);
		(!fits).then(|| format!("must be a whole number from 0{}", if variable.kind == Kind::U16 { " to 65535" } else { "" }))
	};
	match variable.kind {
		Kind::Bool => (!matches!(item, "true" | "false")).then(|| "must be true or false".to_owned()),
		Kind::Enum => (!variable.values.iter().any(|allowed| allowed == item)).then(|| format!("must be one of {}", variable.values.join(", "))),
		Kind::U16 => whole(1 << 16),
		Kind::U32 => whole(1 << 32),
		Kind::U64 | Kind::Usize | Kind::Seconds => whole(1 << 64),
		Kind::Number => item.trim().parse::<f64>().is_err().then(|| "must be a number".to_owned()),
		Kind::SocketAddr => item.parse::<SocketAddr>().is_err().then(|| "must be an address and port, e.g. 0.0.0.0:9464".to_owned()),
		Kind::Path | Kind::String => None,
	}
}

#[cfg(test)]
mod tests {
	use super::{bound_refusal, number, refusal};
	use crate::schema::{
		fixture::{bounds, var},
		Kind,
	};

	#[test]
	fn a_value_is_refused_as_its_binary_would() {
		let switch = var("ON", Kind::Bool);
		assert!(refusal(&switch, "on").is_some() && refusal(&switch, "true").is_none());
		let address = var("ADDR", Kind::SocketAddr);
		assert!(refusal(&address, "[::1]:80").is_none() && refusal(&address, "0.0.0.0").is_some() && refusal(&address, "localhost:1").is_some());
		let port = var("PORT", Kind::U16);
		assert_eq!(refusal(&port, "70000").as_deref(), Some("must be a whole number from 0 to 65535"));
		assert!(refusal(&port, "+80").is_none() && refusal(&port, "").is_some());
		let mut mode = var("MODE", Kind::Enum);
		mode.values = vec!["a".into(), "b".into()];
		assert_eq!(refusal(&mode, "c").as_deref(), Some("must be one of a, b"));
		let mut list = var("LIST", Kind::U32);
		list.delimiter = Some(',');
		assert!(refusal(&list, "1,2").is_none() && refusal(&list, "1,x").is_some());
		assert_eq!((number(60.0), number(0.5)), ("60".to_owned(), "0.5".to_owned()));
	}

	#[test]
	fn a_bound_holds_only_while_every_condition_does() {
		let mut days = var("DAYS", Kind::U64);
		days.bounds = Some(bounds(Some(30.0), Some(100.0), &[("ON", "true"), ("TOKEN", "*")]));
		let off = |_: &str| None;
		assert!(bound_refusal(&days, "1", &off).is_none());
		let on = |name: &str| match name {
			"ON" => Some("true".to_owned()),
			"TOKEN" => Some("t".to_owned()),
			_ => None,
		};
		assert_eq!(bound_refusal(&days, "101", &on).as_deref(), Some("must be from 30 to 100"));
		let blank = |name: &str| if name == "ON" { Some("true".to_owned()) } else { Some(" ".to_owned()) };
		assert!(bound_refusal(&days, "101", &blank).is_none(), "a blank value is not set");
	}
}
