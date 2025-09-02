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
	is_absolute: bool,
}

impl Path {
	pub fn parse(s: &str) -> Result<Self, PathPartError> {
		let is_absolute = s.starts_with('/');
		let stripped = s.trim_matches('/');

		if stripped.is_empty() {
			return Ok(Self { raw: String::new(), is_absolute });
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

		Ok(Self {
			raw: stripped.to_string(),
			is_absolute,
		})
	}

	pub fn from_url_path(path: impl AsRef<str>) -> Result<Self, PathPartError> {
		let path = path.as_ref();
		let decoded = percent_decode(path.as_bytes()).decode_utf8().map_err(|e| PathPartError::NonUnicode {
			path: path.to_string(),
			source: e,
		})?;

		Self::parse(&decoded)
	}

	pub fn is_absolute(&self) -> bool {
		self.is_absolute
	}

	pub fn prefix_match<'a>(&'a self, prefix: &Path) -> Option<impl Iterator<Item = PathPart<'a>>> {
		// Absolute/relative paths should only match with same type
		if self.is_absolute != prefix.is_absolute {
			return None;
		}

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
			Path {
				raw: encoded_segment,
				is_absolute: self.is_absolute,
			}
		} else {
			Path {
				raw: format!("{}{}{}", self.raw, DELIMITER, encoded_segment),
				is_absolute: self.is_absolute,
			}
		}
	}

	pub fn as_ref(&self) -> &str {
		&self.raw
	}

	/// Get the full path string including leading slash for absolute paths
	pub fn to_string_with_prefix(&self) -> String {
		if self.is_absolute && !self.raw.is_empty() {
			format!("/{}", self.raw)
		} else if self.is_absolute {
			"/".to_string()
		} else {
			self.raw.clone()
		}
	}
}

impl Default for Path {
	fn default() -> Self {
		Path {
			raw: String::new(),
			is_absolute: false,
		}
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

		Self { raw, is_absolute: false }
	}
}

impl From<&str> for Path {
	fn from(s: &str) -> Self {
		Self::parse(s).unwrap_or_default()
	}
}

impl From<String> for Path {
	fn from(s: String) -> Self {
		Self::from(s.as_str())
	}
}

impl From<Path> for String {
	fn from(path: Path) -> Self {
		path.to_string_with_prefix()
	}
}

impl std::fmt::Display for Path {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.to_string_with_prefix().fmt(f)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_path_parsing() {
		assert_eq!(Path::parse("/").unwrap().as_ref(), "");
		assert!(Path::parse("/").unwrap().is_absolute());

		assert_eq!(Path::parse("foo/bar/").unwrap().as_ref(), "foo/bar");
		assert!(!Path::parse("foo/bar/").unwrap().is_absolute());

		assert!(Path::parse("foo//bar").is_err());
	}

	#[test]
	fn test_absolute_path_parsing() {
		let abs_path = Path::parse("/app/secrets/client_secret_file.json").unwrap();
		assert!(abs_path.is_absolute());
		assert_eq!(abs_path.as_ref(), "app/secrets/client_secret_file.json");
		assert_eq!(abs_path.to_string_with_prefix(), "/app/secrets/client_secret_file.json");

		let rel_path = Path::parse("app/secrets/client_secret_file.json").unwrap();
		assert!(!rel_path.is_absolute());
		assert_eq!(rel_path.as_ref(), "app/secrets/client_secret_file.json");
		assert_eq!(rel_path.to_string_with_prefix(), "app/secrets/client_secret_file.json");
	}

	#[test]
	fn test_from_url_path() {
		assert_eq!(Path::from_url_path("foo%20bar").unwrap().as_ref(), "foo bar");
		assert!(Path::from_url_path("foo/%FF/bar").is_err());

		let abs_url = Path::from_url_path("/foo%20bar").unwrap();
		assert!(abs_url.is_absolute());
		assert_eq!(abs_url.to_string_with_prefix(), "/foo bar");
	}

	#[test]
	fn test_prefix_matching() {
		let path = Path::from("/foo/bar/baz");
		let abs_prefix = Path::from("/foo/bar");
		let rel_prefix = Path::from("foo/bar");

		assert!(path.prefix_matches(&abs_prefix));
		assert!(!path.prefix_matches(&rel_prefix)); // Different absolute/relative

		let rel_path = Path::from("foo/bar/baz");
		assert!(rel_path.prefix_matches(&rel_prefix));
		assert!(!rel_path.prefix_matches(&abs_prefix));
	}

	#[test]
	fn test_filename_and_extension() {
		let path = Path::from("/foo/bar.txt");
		assert_eq!(path.filename(), Some("bar.txt"));
		assert_eq!(path.extension(), Some("txt"));
		assert!(path.is_absolute());
	}

	#[test]
	fn test_child() {
		let abs_path = Path::from("/foo/bar");
		let child = abs_path.child("baz.txt");
		assert_eq!(child.as_ref(), "foo/bar/baz.txt");
		assert!(child.is_absolute());
		assert_eq!(child.to_string_with_prefix(), "/foo/bar/baz.txt");

		let rel_path = Path::from("foo/bar");
		let rel_child = rel_path.child("baz.txt");
		assert_eq!(rel_child.as_ref(), "foo/bar/baz.txt");
		assert!(!rel_child.is_absolute());
		assert_eq!(rel_child.to_string_with_prefix(), "foo/bar/baz.txt");
	}

	#[test]
	fn test_from_iter() {
		let path = Path::from_iter(vec!["foo", "bar", "baz.txt"]);
		assert_eq!(path.as_ref(), "foo/bar/baz.txt");
		assert!(!path.is_absolute()); // FromIterator creates relative paths
	}

	#[test]
	fn test_display_and_string_conversion() {
		let abs_path = Path::from("/app/config");
		assert_eq!(format!("{}", abs_path), "/app/config");
		assert_eq!(String::from(abs_path.clone()), "/app/config");

		let rel_path = Path::from("app/config");
		assert_eq!(format!("{}", rel_path), "app/config");
		assert_eq!(String::from(rel_path.clone()), "app/config");
	}

	#[test]
	fn test_root_path() {
		let root = Path::parse("/").unwrap();
		assert!(root.is_absolute());
		assert_eq!(root.as_ref(), "");
		assert_eq!(root.to_string_with_prefix(), "/");
		assert_eq!(format!("{}", root), "/");
	}

	#[test]
	fn parse_multiple_leading_slashes() {
		let err = Path::parse("//foo/bar").unwrap_err();
		assert!(matches!(err, PathPartError::EmptySegment { .. }));

		let path = Path::parse("/foo/bar/").unwrap();
		assert_eq!(path.as_ref(), "foo/bar");
		assert!(path.is_absolute());
	}

	#[test]
	fn parse_invalid_characters() {
		let err = Path::parse("foo/\x7F/bar").unwrap_err(); // Using a control character
		assert!(matches!(err, PathPartError::NonUnicode { .. }));
	}

	#[test]
	fn prefix_match_empty_prefix() {
		let existing_path = Path::from("/apple/bear/cow/dog/egg.json");
		let empty_abs_prefix = Path::from("/");
		let empty_rel_prefix = Path::from("");

		let parts: Vec<_> = existing_path.prefix_match(&empty_abs_prefix).unwrap().collect();
		assert_eq!(parts.len(), 5); // Should return all parts

		// Should not match relative empty prefix with absolute path
		assert!(existing_path.prefix_match(&empty_rel_prefix).is_none());
	}

	#[test]
	fn filename_no_extension() {
		let a = Path::from("/foo/bar/");
		assert_eq!(a.filename(), Some("bar"));
		assert!(a.is_absolute());
	}

	#[test]
	fn complex_encoded_url_path() {
		let path = Path::from_url_path("/foo%20bar/baz%20qux").unwrap();
		assert_eq!(path.raw, "foo bar/baz qux");
		assert!(path.is_absolute());
		assert_eq!(path.to_string_with_prefix(), "/foo bar/baz qux");
	}

	#[test]
	fn parse_with_multiple_segments_and_dots() {
		let path = Path::from("/foo.bar/baz.qux");
		assert_eq!(path.filename(), Some("baz.qux"));
		assert_eq!(path.extension(), Some("qux"));
		assert!(path.is_absolute());
	}

	#[test]
	fn from_url_path() {
		let a = Path::from_url_path("foo%20bar").unwrap();
		let b = Path::from_url_path("foo/%2E%2E/bar").unwrap_err();
		let c = Path::from_url_path("foo%2F%252E%252E%2Fbar").unwrap();
		let d = Path::from_url_path("foo/%252E%252E/bar").unwrap();
		let e = Path::from_url_path("%48%45%4C%4C%4F").unwrap();
		let f = Path::from_url_path("foo/%FF/as").unwrap_err();
		let g = Path::from_url_path("/foo%20bar").unwrap(); // Test absolute URL path

		assert_eq!(a.raw, "foo bar");
		assert!(!a.is_absolute());
		assert!(matches!(b, PathPartError::BadSegment { .. }));
		assert_eq!(c.raw, "foo/%2E%2E/bar");
		assert_eq!(d.raw, "foo/%2E%2E/bar");
		assert_eq!(e.raw, "HELLO");
		assert!(matches!(f, PathPartError::NonUnicode { .. }));
		assert_eq!(g.raw, "foo bar");
		assert!(g.is_absolute());
	}

	#[test]
	fn filename_from_path() {
		let a = Path::from("/foo/bar");
		let b = Path::from("/foo/bar.baz");
		let c = Path::from("/foo.bar/baz");

		assert_eq!(a.filename(), Some("bar"));
		assert_eq!(b.filename(), Some("bar.baz"));
		assert_eq!(c.filename(), Some("baz"));
		assert!(a.is_absolute() && b.is_absolute() && c.is_absolute());
	}

	#[test]
	fn file_extension() {
		let a = Path::from("/foo/bar");
		let b = Path::from("/foo/bar.baz");
		let c = Path::from("/foo.bar/baz");
		let d = Path::from("/foo.bar/baz.qux");

		assert_eq!(a.extension(), None);
		assert_eq!(b.extension(), Some("baz"));
		assert_eq!(c.extension(), None);
		assert_eq!(d.extension(), Some("qux"));
		assert!(a.is_absolute() && b.is_absolute() && c.is_absolute() && d.is_absolute());
	}

	#[test]
	fn docker_style_absolute_paths() {
		let client_secret = Path::parse("/app/secrets/client_secret_file.json").unwrap();
		assert!(client_secret.is_absolute());
		assert_eq!(client_secret.filename(), Some("client_secret_file.json"));
		assert_eq!(client_secret.extension(), Some("json"));
		assert_eq!(format!("{}", client_secret), "/app/secrets/client_secret_file.json");

		let child = client_secret.child("backup");
		assert_eq!(format!("{}", child), "/app/secrets/client_secret_file.json/backup");
		assert!(child.is_absolute());
	}
}
