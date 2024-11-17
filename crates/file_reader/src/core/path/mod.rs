use crate::config::PathPartError;
use percent_encoding::percent_decode;
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
use std::iter::FromIterator;

pub mod part;

pub use part::PathPart;

pub const DELIMITER: &str = "/";
#[allow(dead_code)]
const DELIMITER_BYTE: u8 = DELIMITER.as_bytes()[0];

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct Path {
	raw: String,
}

impl Path {
	pub fn parse(s: &str) -> Result<Self, PathPartError> {
		let stripped = s.trim_matches('/');
		if stripped.is_empty() {
			return Ok(Default::default());
		}

		for segment in stripped.split(DELIMITER) {
			if segment.is_empty() {
				return Err(PathPartError::EmptySegment { path: s.to_string() });
			}
			PathPart::parse(segment).map_err(|e| PathPartError::BadSegment {
				path: s.to_string(),
				source: Box::new(e),
			})?;
		}

		Ok(Self { raw: stripped.to_string() })
	}

	pub fn from_url_path(path: impl AsRef<str>) -> Result<Self, PathPartError> {
		let path = path.as_ref();
		let decoded = percent_decode(path.as_bytes()).decode_utf8().map_err(|e| PathPartError::NonUnicode {
			path: path.to_string(),
			source: e,
		})?;

		Self::parse(&decoded)
	}

	// The rest of the methods remain unchanged
	pub fn prefix_match<'a>(&'a self, prefix: &Path) -> Option<impl Iterator<Item = PathPart<'a>>> {
		let mut stripped = self.raw.strip_prefix(&prefix.raw)?;
		if !stripped.is_empty() && !prefix.raw.is_empty() {
			stripped = stripped.strip_prefix(DELIMITER)?;
		}
		Some(stripped.split(DELIMITER).map(PathPart::from))
	}

	pub fn prefix_matches(&self, prefix: &Path) -> bool {
		self.prefix_match(prefix).is_some()
	}

	pub fn parts(&self) -> impl Iterator<Item = PathPart<'_>> {
		self.raw.split(DELIMITER).map(PathPart::from)
	}

	pub fn filename(&self) -> Option<&str> {
		self.raw.rsplit(DELIMITER).next()
	}

	pub fn extension(&self) -> Option<&str> {
		self.filename().and_then(|f| f.rsplit_once('.')).map(|(_, ext)| ext)
	}

	pub fn child<'a>(&self, segment: impl Into<PathPart<'a>>) -> Self {
		let encoded_segment = segment.into().raw.to_string();
		if self.raw.is_empty() {
			Path { raw: encoded_segment }
		} else {
			Path {
				raw: format!("{}{}{}", self.raw, DELIMITER, encoded_segment),
			}
		}
	}

	pub fn as_ref(&self) -> &str {
		&self.raw
	}
}

impl Default for Path {
	fn default() -> Self {
		Path { raw: String::new() }
	}
}

impl<'a, I> FromIterator<I> for Path
where
	I: Into<PathPart<'a>>,
{
	fn from_iter<T: IntoIterator<Item = I>>(iter: T) -> Self {
		let raw = T::into_iter(iter)
			.map(|s| s.into())
			.filter(|s| !s.raw.is_empty())
			.map(|s| s.raw.to_string())
			.collect::<Vec<_>>()
			.join(DELIMITER);

		Self { raw }
	}
}

impl From<&str> for Path {
	fn from(s: &str) -> Self {
		Self::from_iter(s.split(DELIMITER))
	}
}

impl From<String> for Path {
	fn from(s: String) -> Self {
		Self::from_iter(s.split(DELIMITER))
	}
}

impl From<Path> for String {
	fn from(path: Path) -> Self {
		path.raw
	}
}

impl std::fmt::Display for Path {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.raw.fmt(f)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_path_parsing() {
		assert_eq!(Path::parse("/").unwrap().as_ref(), "");
		assert_eq!(Path::parse("foo/bar/").unwrap().as_ref(), "foo/bar");
		assert!(Path::parse("foo//bar").is_err());
	}

	#[test]
	fn test_from_url_path() {
		assert_eq!(Path::from_url_path("foo%20bar").unwrap().as_ref(), "foo bar");
		assert!(Path::from_url_path("foo/%FF/bar").is_err());
	}

	#[test]
	fn test_prefix_matching() {
		let path = Path::from("foo/bar/baz");
		assert!(path.prefix_matches(&Path::from("foo/bar")));
		assert!(!path.prefix_matches(&Path::from("foo/baz")));
	}

	#[test]
	fn test_filename_and_extension() {
		let path = Path::from("foo/bar.txt");
		assert_eq!(path.filename(), Some("bar.txt"));
		assert_eq!(path.extension(), Some("txt"));
	}

	#[test]
	fn test_child() {
		let path = Path::from("foo/bar");
		let child = path.child("baz.txt");
		assert_eq!(child.as_ref(), "foo/bar/baz.txt");
	}

	#[test]
	fn test_from_iter() {
		let path = Path::from_iter(vec!["foo", "bar", "baz.txt"]);
		assert_eq!(path.as_ref(), "foo/bar/baz.txt");
	}

	#[test]
	fn parse_multiple_leading_slashes() {
		let err = Path::parse("//foo/bar").unwrap_err();
		assert!(matches!(err, PathPartError::EmptySegment { .. }));

		let path = Path::parse("/foo/bar/").unwrap();
		assert_eq!(path.as_ref(), "foo/bar");
	}

	#[test]
	fn parse_invalid_characters() {
		let err = Path::parse("foo/\x7F/bar").unwrap_err(); // Using a control character
		assert!(matches!(err, PathPartError::NonUnicode { .. }));
	}

	#[test]
	fn prefix_match_empty_prefix() {
		let existing_path = Path::from("apple/bear/cow/dog/egg.json");
		let prefix = Path::from("");

		let parts: Vec<_> = existing_path.prefix_match(&prefix).unwrap().collect();
		assert_eq!(parts.len(), 5); // Should return all parts
	}

	#[test]
	fn filename_no_extension() {
		let a = Path::from("foo/bar/");
		assert_eq!(a.filename(), Some("bar"));
	}

	#[test]
	fn complex_encoded_url_path() {
		let path = Path::from_url_path("foo%20bar/baz%20qux").unwrap();
		assert_eq!(path.raw, "foo bar/baz qux");
	}

	#[test]
	fn parse_with_multiple_segments_and_dots() {
		let path = Path::from("foo.bar/baz.qux");
		assert_eq!(path.filename(), Some("baz.qux"));
		assert_eq!(path.extension(), Some("qux"));
	}

	#[test]
	fn from_url_path() {
		let a = Path::from_url_path("foo%20bar").unwrap();
		let b = Path::from_url_path("foo/%2E%2E/bar").unwrap_err();
		let c = Path::from_url_path("foo%2F%252E%252E%2Fbar").unwrap();
		let d = Path::from_url_path("foo/%252E%252E/bar").unwrap();
		let e = Path::from_url_path("%48%45%4C%4C%4F").unwrap();
		let f = Path::from_url_path("foo/%FF/as").unwrap_err();

		assert_eq!(a.raw, "foo bar");
		assert!(matches!(b, PathPartError::BadSegment { .. }));
		assert_eq!(c.raw, "foo/%2E%2E/bar");
		assert_eq!(d.raw, "foo/%2E%2E/bar");
		assert_eq!(e.raw, "HELLO");
		assert!(matches!(f, PathPartError::NonUnicode { .. }));
	}

	#[test]
	fn filename_from_path() {
		let a = Path::from("foo/bar");
		let b = Path::from("foo/bar.baz");
		let c = Path::from("foo.bar/baz");

		assert_eq!(a.filename(), Some("bar"));
		assert_eq!(b.filename(), Some("bar.baz"));
		assert_eq!(c.filename(), Some("baz"));
	}

	#[test]
	fn file_extension() {
		let a = Path::from("foo/bar");
		let b = Path::from("foo/bar.baz");
		let c = Path::from("foo.bar/baz");
		let d = Path::from("foo.bar/baz.qux");

		assert_eq!(a.extension(), None);
		assert_eq!(b.extension(), Some("baz"));
		assert_eq!(c.extension(), None);
		assert_eq!(d.extension(), Some("qux"));
	}
	// Add more tests as needed
}
