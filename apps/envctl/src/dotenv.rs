//! Dotenv files and `${...}` interpolation, as docker compose and dotenvy
//! read them.

use indexmap::IndexMap;
use regex::Regex;
use std::sync::LazyLock;

static LINE: LazyLock<Regex> = LazyLock::new(|| regex(r"^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$"));
static TRAILING_COMMENT: LazyLock<Regex> = LazyLock::new(|| regex(r"\s#"));

/// A pattern this crate writes; one that does not compile is a bug here.
pub(crate) fn regex(pattern: &str) -> Regex {
	match Regex::new(pattern) {
		Ok(regex) => regex,
		Err(err) => unreachable!("{pattern}: {err}"),
	}
}

/// One value of a dotenv file: whether it is literal (single-quoted, so
/// never interpolated).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
	pub(crate) value: String,
	pub(crate) literal: bool,
}

pub(crate) type Entries = IndexMap<String, Entry>;

/// The name and raw value of a `NAME=value` line, or None.
pub(crate) fn assignment(line: &str) -> Option<(String, String)> {
	if line.trim_start().starts_with('#') {
		return None;
	}
	let captures = LINE.captures(line)?;
	Some((captures[1].to_owned(), captures[2].to_owned()))
}

/// Each variable a dotenv file sets, as compose and dotenvy read it. A later
/// line wins over an earlier one, as it does for both.
pub(crate) fn parse_entries(text: &str) -> Entries {
	let mut entries = Entries::new();
	for line in text.lines() {
		let Some((name, raw)) = assignment(line) else { continue };
		let entry = raw
			.strip_prefix('\'')
			.and_then(single_quoted)
			.map_or_else(|| interpolated(&raw), |value| Entry { value, literal: true });
		// A repeated name keeps its first place, as a Python dict and compose's
		// own reader do; its last value wins.
		entries.insert(name, entry);
	}
	entries
}

/// The body of a single-quoted value, from just after its opening quote,
/// as compose reads it: literal, but for `\'`, a quote. None when it never
/// closes.
fn single_quoted(rest: &str) -> Option<String> {
	let mut out = String::new();
	let mut characters = rest.chars();
	while let Some(character) = characters.next() {
		match character {
			'\\' if characters.clone().next() == Some('\'') => {
				out.push('\'');
				characters.next();
			}
			'\'' => return Some(out),
			other => out.push(other),
		}
	}
	None
}

/// A value that is not single-quoted: double-quoted, or bare up to a
/// trailing ` #` comment.
fn interpolated(raw: &str) -> Entry {
	let value = raw.strip_prefix('"').map_or_else(
		|| TRAILING_COMMENT.splitn(raw, 2).next().unwrap_or_default().trim().to_owned(),
		|rest| unescape(double_quoted(rest)),
	);
	Entry { value, literal: false }
}

/// The body of a double-quoted value, up to its closing quote (all of it,
/// when there is none).
fn double_quoted(rest: &str) -> &str {
	let mut escaped = false;
	for (index, character) in rest.char_indices() {
		match character {
			'\\' if !escaped => escaped = true,
			'"' if !escaped => return &rest[..index],
			_ => escaped = false,
		}
	}
	rest
}

fn unescape(body: &str) -> String {
	let mut out = String::with_capacity(body.len());
	let mut characters = body.chars().peekable();
	while let Some(character) = characters.next() {
		if character == '\\' && matches!(characters.peek(), Some('"' | '\\' | '$')) {
			if let Some(next) = characters.next() {
				out.push(next);
			}
		} else {
			out.push(character);
		}
	}
	out
}

/// The variables a dotenv file sets, as compose and dotenvy read them.
pub(crate) fn parse(text: &str) -> IndexMap<String, String> {
	parse_entries(text).into_iter().map(|(name, entry)| (name, entry.value)).collect()
}

/// One piece of a value compose interpolates.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Token<'a> {
	Text(&'a str),
	/// `$$`, a literal `$`.
	Dollar,
	/// `$NAME`.
	Bare(&'a str),
	/// `${NAME}`, `${NAME:-default}`, `${NAME?message}` and the rest.
	Braced {
		name: &'a str,
		colon: bool,
		operator: Option<char>,
		argument: &'a str,
	},
}

const fn is_name_start(character: u8) -> bool {
	character.is_ascii_alphabetic() || character == b'_'
}

const fn is_name(character: u8) -> bool {
	character.is_ascii_alphanumeric() || character == b'_'
}

/// `text` as compose reads it: literal text, `$$`, `$NAME` and `${...}`.
pub(crate) fn tokens(text: &str) -> Vec<Token<'_>> {
	let bytes = text.as_bytes();
	let mut out = Vec::new();
	let mut start = 0;
	let mut index = 0;
	while index < bytes.len() {
		if bytes[index] != b'$' {
			index += 1;
			continue;
		}
		let parsed = match bytes.get(index + 1) {
			Some(b'$') => Some((Token::Dollar, index + 2)),
			Some(b'{') => braced(text, index + 2),
			Some(&next) if is_name_start(next) => {
				let end = (index + 1..bytes.len()).find(|&at| !is_name(bytes[at])).unwrap_or(bytes.len());
				Some((Token::Bare(&text[index + 1..end]), end))
			}
			_ => None,
		};
		if let Some((token, end)) = parsed {
			if start < index {
				out.push(Token::Text(&text[start..index]));
			}
			out.push(token);
			start = end;
			index = end;
		} else {
			index += 1;
		}
	}
	if start < bytes.len() {
		out.push(Token::Text(&text[start..]));
	}
	out
}

/// `${...}` from just after its brace: a name, then `:`? and one of `-?+`
/// with an argument that may hold one level of `{...}`, then `}`.
fn braced(text: &str, from: usize) -> Option<(Token<'_>, usize)> {
	let bytes = text.as_bytes();
	if !bytes.get(from).copied().is_some_and(is_name_start) {
		return None;
	}
	let name_end = (from..bytes.len()).find(|&at| !is_name(bytes[at])).unwrap_or(bytes.len());
	let name = &text[from..name_end];
	let mut at = name_end;
	if bytes.get(at) == Some(&b'}') {
		return Some((
			Token::Braced {
				name,
				colon: false,
				operator: None,
				argument: "",
			},
			at + 1,
		));
	}
	let colon = bytes.get(at) == Some(&b':');
	if colon {
		at += 1;
	}
	let operator = match bytes.get(at) {
		Some(&operator @ (b'-' | b'?' | b'+')) => char::from(operator),
		_ => return None,
	};
	at += 1;
	let argument_start = at;
	let mut depth = 0;
	while at < bytes.len() {
		match bytes[at] {
			b'{' if depth == 0 => depth = 1,
			b'{' => return None,
			b'}' if depth == 1 => depth = 0,
			b'}' => {
				return Some((
					Token::Braced {
						name,
						colon,
						operator: Some(operator),
						argument: &text[argument_start..at],
					},
					at + 1,
				));
			}
			_ => {}
		}
		at += 1;
	}
	None
}

/// `value` as compose interpolates it from `given`, an unset `${NAME:?}`
/// becoming empty.
pub(crate) fn interpolate(value: &str, given: &dyn Fn(&str) -> Option<String>) -> String {
	let mut out = String::new();
	for token in tokens(value) {
		match token {
			Token::Text(text) => out.push_str(text),
			Token::Dollar => out.push('$'),
			Token::Bare(name) => out.push_str(&given(name).unwrap_or_default()),
			Token::Braced { name, colon, operator, argument } => {
				let current = given(name);
				let unset = current.as_deref().is_none_or(|current| colon && current.is_empty());
				match operator {
					Some('-') if unset => out.push_str(&interpolate(argument, given)),
					Some('+') if !unset => out.push_str(&interpolate(argument, given)),
					Some('+') => {}
					_ => out.push_str(&current.unwrap_or_default()),
				}
			}
		}
	}
	out
}

#[cfg(test)]
// `${...}` is compose's syntax under test here, not a format argument.
#[allow(clippy::literal_string_with_formatting_args)]
mod tests {
	use super::{interpolate, parse, parse_entries, tokens, Entry, Token};

	fn given<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
		move |name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
	}

	#[test]
	fn values_read_as_compose_and_dotenvy_read_them() {
		let parsed = parse("A=\"x # y\"\nB=z # c\nexport C='q'\nD=\"3000\" # d\nE=\"a\\\"b\"\n# F=no\nG=1\nG=2\n");
		let pairs: Vec<(&str, &str)> = parsed.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
		assert_eq!(pairs, [("A", "x # y"), ("B", "z"), ("C", "q"), ("D", "3000"), ("E", "a\"b"), ("G", "2")]);
		assert_eq!(parse("Q='it\\'s'\nB='a\\\\b'\n")["Q"], "it's", "\\' is a quote, as compose reads it");
		assert_eq!(parse("B='a\\\\b'\n")["B"], "a\\\\b", "any other backslash is literal");
		assert_eq!(
			parse_entries("K='pa$word'\n")["K"],
			Entry {
				value: "pa$word".into(),
				literal: true
			}
		);
	}

	#[test]
	fn interpolation_follows_compose() {
		let values = [("A", ""), ("B", ""), ("C", "1"), ("D", "d")];
		assert_eq!(interpolate("${A:-x}/${B-y}/${C:+z}/$D/$$E/${F:?m}", &given(&values)), "x//z/d/$E/");
		assert_eq!(interpolate("${X:-${Y:-deep}}/x", &given(&[])), "deep/x");
		assert!(tokens("$${NOT}").iter().all(|token| !matches!(token, Token::Braced { .. })));
	}
}
