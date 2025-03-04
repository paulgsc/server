use crate::config::PathPartError;
/// Copyright [2024] [pgdev]
///
/// Licensed under the Apache License, Version 2.0 (the "License");
/// you may not use this file except in compliance with the License.
/// You may obtain a copy of the License at
///
///     http://www.apache.org/licenses/LICENSE-2.0
///
/// Unless required by applicable law or agreed to in writing, software
/// distributed under the License is distributed on an "AS IS" BASIS,
/// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
/// See the License for the specific language governing permissions and
/// limitations under the License.
///
// This file has been modified from its original version.
//
use percent_encoding::{percent_encode, AsciiSet, CONTROLS};
use std::borrow::Cow;

/// The PathPart type exists to validate the directory/file names that form part
/// of a path.
#[derive(Clone, PartialEq, Eq, Debug, Default, Hash)]
pub struct PathPart<'a> {
	pub(super) raw: Cow<'a, str>,
}

impl<'a> PathPart<'a> {
	/// Parse the provided path segment as a [`PathPart`] returning an error if invalid
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

		Ok(Self {
			raw: percent_encode(segment.as_bytes(), INVALID).into(),
		})
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
	fn from(v: &'a str) -> Self {
		PathPart::parse(v).unwrap()
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
