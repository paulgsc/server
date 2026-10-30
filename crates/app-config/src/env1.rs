//! ENV1's proof over one binary's `Config` (README.md, "Configuration").
//!
//! Each binary's `every_variable_that_does_not_parse_is_refused_by_name` test
//! calls [`assert_every_variable_is_refused_by_name`] with its own lists, so
//! the walk is written once and every binary is held to the same rule.

use clap::{error::ErrorKind, Parser};
use std::{any::TypeId, path::PathBuf};

/// Holds ENV1 for every variable `C` declares.
///
/// One whose value can fail to parse is refused, by an error naming it, and
/// one that can't is listed as free text. A variable in neither list, or a
/// listed one no longer declared, fails.
///
/// `refused` pairs a variable with a value it must refuse; a variable may be
/// listed more than once. Clap applies the same parser to the flag and the
/// variable, so the flag stands in for the variable without touching the
/// process environment. Its error names the argument by its value name, never
/// by the variable a value came from, so each variable must be its own value
/// name.
///
/// # Panics
///
/// When any of the above does not hold; it is meant for a test.
pub fn assert_every_variable_is_refused_by_name<C: Parser>(binary: &str, refused: &[(&str, &str)], free_text: &[&str]) {
	let mut seen = Vec::new();
	for arg in C::command().get_arguments() {
		let Some(env) = arg.get_env().and_then(std::ffi::OsStr::to_str) else { continue };
		seen.push(env.to_owned());
		let value_names: Vec<&str> = arg.get_value_names().unwrap_or_default().iter().map(clap::builder::Str::as_str).collect();
		assert_eq!(value_names, [env], "{env} is not its own value name, so its errors do not name it");
		let Some(long) = arg.get_long() else { panic!("{env} has no flag to test it with") };
		let bad_values: Vec<&str> = refused.iter().filter(|(name, _)| *name == env).map(|(_, bad)| *bad).collect();
		for bad in &bad_values {
			let flag = String::from("--") + long + "=" + bad;
			let Err(err) = C::try_parse_from([binary, &flag]) else {
				panic!("{env}={bad} was accepted")
			};
			assert!(matches!(err.kind(), ErrorKind::ValueValidation | ErrorKind::InvalidValue), "{env}={bad}: {err}");
			assert!(err.to_string().contains(env), "{env}={bad}: {err}");
		}
		if bad_values.is_empty() {
			assert!(free_text.contains(&env), "{env} is in neither REFUSED nor FREE_TEXT");
			let parser = arg.get_value_parser().type_id();
			assert!(
				parser == TypeId::of::<String>() || parser == TypeId::of::<PathBuf>(),
				"{env} is free text, so its parser must be a plain String or PathBuf"
			);
		}
	}
	let mut declared: Vec<&str> = refused.iter().map(|(name, _)| *name).chain(free_text.iter().copied()).collect();
	declared.sort_unstable();
	declared.dedup();
	seen.sort_unstable();
	assert_eq!(seen, declared, "a listed variable is no longer declared");
}
