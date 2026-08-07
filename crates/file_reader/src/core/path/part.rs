use crate::config::PathPartError;
// Copyright [2024] [pgdev]
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// This file has been modified from its original version.
//
use percent_encoding::{percent_encode, AsciiSet, CONTROLS};
use std::borrow::Cow;

/// The `PathPart` type exists to validate the directory/file names that form
/// part of a path.
#[derive(Clone, PartialEq, Eq, Debug, Default, Hash)]
pub struct PathPart<'a> {
	pub(super) raw: Cow<'a, str>,
}

impl<'a> PathPart<'a> {
	/// Parse the provided path segment as a [`PathPart`] returning an error if invalid
	///
	/// The segment is expected to be *already* encoded — this is the validating
	/// counterpart to [`From<&str>`], which encodes. A segment that parses is
	/// kept verbatim; encoding it again here would turn its `%` escapes into
	/// literal `%25` and change the path.
	///
	/// # Errors
	///
	/// Returns [`PathPartError::IllegalCharacter`] if the segment is `.` or
	/// `..`, contains an ASCII control character or the [`DELIMITER`], or
	/// contains a `%` that does not begin a well-formed two-digit hex escape.
	///
	/// [`DELIMITER`]: super::DELIMITER
	pub fn parse(segment: &'a str) -> Result<Self, PathPartError> {
		if segment == "." || segment == ".." {
			return Err(PathPartError::IllegalCharacter {
				segment: segment.to_string(),
				illegal: segment.to_string(),
			});
		}

		for c in segment.chars() {
			if c.is_ascii_control() || c == '/' {
				return Err(PathPartError::IllegalCharacter {
					segment: segment.to_string(),
					illegal: c.to_string(),
				});
			}
		}

		// A bare `%` is not a legal encoded segment: it either starts an escape
		// or it should have been written `%25`. Anything else would decode
		// differently than it reads.
		let bytes = segment.as_bytes();
		for (i, byte) in bytes.iter().enumerate() {
			if *byte != b'%' {
				continue;
			}
			let well_formed = matches!((bytes.get(i + 1), bytes.get(i + 2)), (Some(hi), Some(lo)) if hi.is_ascii_hexdigit() && lo.is_ascii_hexdigit());
			if !well_formed {
				return Err(PathPartError::IllegalCharacter {
					segment: segment.to_string(),
					illegal: segment[i..].chars().take(3).collect(),
				});
			}
		}

		Ok(Self { raw: segment.into() })
	}
}

/// Characters we want to encode.
const INVALID: &AsciiSet = &CONTROLS
	.add(b'/') // Ensure this is included
	.add(b'\\')
	.add(b'{')
	.add(b'^')
	.add(b'}')
	.add(b'%')
	.add(b'`')
	.add(b']')
	.add(b'"')
	.add(b'>')
	.add(b'[')
	.add(b'~')
	.add(b'<')
	.add(b'#')
	.add(b'|')
	.add(b'\r')
	.add(b'\n')
	.add(b'*')
	.add(b'?');

impl<'a> From<&'a str> for PathPart<'a> {
	/// Encode an unvalidated segment. This conversion is infallible by
	/// construction: every input has an encoding. `.` and `..` are special-cased
	/// rather than left alone, so that a segment which would let a caller walk
	/// out of its prefix cannot survive the round trip.
	fn from(v: &'a str) -> Self {
		let raw = match v {
			"." => "%2E".into(),
			".." => "%2E%2E".into(),
			other => percent_encode(other.as_bytes(), INVALID).into(),
		};
		Self { raw }
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn path_part_delimiter_gets_encoded() {
		let part: PathPart<'_> = "foo/bar".into();
		assert_eq!(part.raw, "foo%2Fbar");
	}

	#[test]
	fn path_part_given_already_encoded_string() {
		let part: PathPart<'_> = "foo%2Fbar".into();
		assert_eq!(part.raw, "foo%252Fbar");
	}

	#[test]
	fn path_part_cant_be_one_dot() {
		let part: PathPart<'_> = ".".into();
		assert_eq!(part.raw, "%2E");
	}

	#[test]
	fn path_part_cant_be_two_dots() {
		let part: PathPart<'_> = "..".into();
		assert_eq!(part.raw, "%2E%2E");
	}

	#[test]
	fn path_part_parse() {
		PathPart::parse("foo").unwrap();
		PathPart::parse("foo/bar").unwrap_err();
		// Test percent-encoded path
		PathPart::parse("foo%2Fbar").unwrap();
		PathPart::parse("L%3ABC.parquet").unwrap();
		// Test path containing bad escape sequence
		PathPart::parse("%Z").unwrap_err();
		PathPart::parse("%%").unwrap_err();
	}
}
