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

use std::borrow::Cow;
use std::iter::FromIterator;
use std::path::PathBuf;
use percent_encoding::{percent_decode_str, percent_encode, AsciiSet, CONTROLS};

const PATH_SEGMENT_ENCODE_SET: &AsciiSet = &CONTROLS.add(b'/').add(b'%');

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    raw: String,
}

#[derive(Debug)]
pub enum Error {
    EmptySegment { position: usize },
    BadSegment { segment: String },
    NonUnicode { segment: String },
}

impl Path {
    pub fn from_iter<I, S>(segments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let encoded: Vec<String> = segments
            .into_iter()
            .map(|s| percent_encode(s.as_ref().as_bytes(), PATH_SEGMENT_ENCODE_SET).to_string())
            .collect();
        Path {
            raw: encoded.join("/"),
        }
    }

    pub fn parse(s: &str) -> Result<Self, Error> {
        if s.is_empty() || s == "/" {
            return Ok(Path { raw: String::new() });
        }

        let trimmed = s.trim_matches('/');
        let segments: Vec<&str> = trimmed.split('/').collect();

        for (i, segment) in segments.iter().enumerate() {
            if segment.is_empty() {
                return Err(Error::EmptySegment { position: i });
            }
        }

        Ok(Path {
            raw: trimmed.to_string(),
        })
    }

    pub fn from_url_path(s: &str) -> Result<Self, Error> {
        let decoded = percent_decode_str(s)
            .decode_utf8()
            .map_err(|_| Error::NonUnicode {
                segment: s.to_string(),
            })?;

        let path = Path::parse(&decoded)?;

        // Check for ".." segments
        if path.raw.contains("/..") || path.raw.starts_with("..") {
            return Err(Error::BadSegment {
                segment: "..".to_string(),
            });
        }

        Ok(path)
    }

    pub fn prefix_match<'a>(&'a self, prefix: &Path) -> Option<impl Iterator<Item = PathPart<'a>>> {
        if self.prefix_matches(prefix) {
            let remaining = &self.raw[prefix.raw.len()..];
            let remaining = remaining.trim_start_matches('/');
            Some(remaining.split('/').map(PathPart::from))
        } else {
            None
        }
    }

    pub fn prefix_matches(&self, prefix: &Path) -> bool {
        if prefix.raw.is_empty() {
            return true;
        }
        self.raw == prefix.raw || self.raw.starts_with(&(prefix.raw.clone() + "/"))
    }

    pub fn child(&self, segment: &str) -> Self {
        let encoded_segment = percent_encode(segment.as_bytes(), PATH_SEGMENT_ENCODE_SET).to_string();
        if self.raw.is_empty() {
            Path { raw: encoded_segment }
        } else {
            Path {
                raw: format!("{}/{}", self.raw, encoded_segment),
            }
        }
    }

    pub fn as_ref(&self) -> &str {
        &self.raw
    }

    pub fn parts(&self) -> impl Iterator<Item = PathPart<'_>> {
        self.raw.split('/').map(PathPart::from)
    }

    pub fn filename(&self) -> Option<&str> {
        self.raw.rsplit('/').next()
    }

    pub fn extension(&self) -> Option<&str> {
        self.filename()?.rsplit('.').next()
    }
}

impl From<&str> for Path {
    fn from(s: &str) -> Self {
        Path::parse(s).unwrap_or_else(|_| Path { raw: s.to_string() })
    }
}

impl Default for Path {
    fn default() -> Self {
        Path { raw: String::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_creation_from_iter() {
        let path = Path::from_iter(vec!["folder", "subfolder", "file.txt"]);
        assert_eq!(path.raw, "folder/subfolder/file.txt");

        let empty_path = Path::from_iter(vec![]);
        assert_eq!(empty_path.raw, "");

        let path_with_special_chars = Path::from_iter(vec!["file name", "with spaces", "and&special#chars"]);
        assert_eq!(path_with_special_chars.raw, "file%20name/with%20spaces/and%26special%23chars");
    }

    #[test]
    fn test_path_parsing() {
        assert_eq!(Path::parse("folder/subfolder/file.txt").unwrap().raw, "folder/subfolder/file.txt");
        assert_eq!(Path::parse("/folder/subfolder/file.txt").unwrap().raw, "folder/subfolder/file.txt");
        assert_eq!(Path::parse("folder/subfolder/").unwrap().raw, "folder/subfolder");

        assert!(Path::parse("").is_ok());
        assert!(Path::parse("/").is_ok());

        // Test for empty segment
        match Path::parse("folder//file.txt") {
            Err(Error::EmptySegment { position }) => assert_eq!(position, 1),
            _ => panic!("Expected an empty segment error"),
        }

        // Test for bad segment
        match Path::parse("folder/../file.txt") {
            Err(Error::BadSegment { segment }) => assert_eq!(segment, ".."),
            _ => panic!("Expected a bad segment error"),
        }
    }

    #[test]
    fn test_path_from_url_path() {
        let path = Path::from_url_path("folder%20with%20spaces/file.txt").unwrap();
        assert_eq!(path.raw, "folder with spaces/file.txt");

        // Test for non-UTF-8 segment
        match Path::from_url_path("%FF") {
            Err(Error::NonUnicode { segment }) => assert_eq!(segment, "%FF"),
            _ => panic!("Expected a non-UTF-8 error"),
        }

        // Test for bad segment with ".."
        match Path::from_url_path("folder/..") {
            Err(Error::BadSegment { segment }) => assert_eq!(segment, ".."),
            _ => panic!("Expected a bad segment error"),
        }
    }

    #[test]
    fn test_prefix_matching() {
        let base_path = Path::from("folder/subfolder");
        let prefix_path = Path::from("folder");

        assert!(base_path.prefix_matches(&prefix_path));

        // Testing prefix match
        if let Some(mut parts) = base_path.prefix_match(&prefix_path) {
            assert_eq!(parts.next().unwrap().0, "subfolder");
        } else {
            panic!("Expected a prefix match to succeed");
        }

        // Testing non-matching prefix
        let non_matching_prefix = Path::from("not_in_path");
        assert!(!base_path.prefix_matches(&non_matching_prefix));
        assert!(base_path.prefix_match(&non_matching_prefix).is_none());
    }

    #[test]
    fn test_child_path_creation() {
        let base_path = Path::from("folder/subfolder");
        let child_path = base_path.child("newfile.txt");
        assert_eq!(child_path.raw, "folder/subfolder/newfile.txt");

        let new_base_path = Path::from("");
        let child_of_empty = new_base_path.child("newfile.txt");
        assert_eq!(child_of_empty.raw, "newfile.txt");
    }

    #[test]
    fn test_path_parts() {
        let path = Path::from("folder/subfolder/file.txt");
        let parts: Vec<String> = path.parts().map(|part| part.0.to_string()).collect();
        assert_eq!(parts, vec!["folder", "subfolder", "file.txt"]);
    }

    #[test]
    fn test_filename_and_extension() {
        let path = Path::from("folder/subfolder/file.txt");
        assert_eq!(path.filename(), Some("file.txt"));
        assert_eq!(path.extension(), Some("txt"));

        let path_without_extension = Path::from("folder/subfolder/file");
        assert_eq!(path_without_extension.filename(), Some("file"));
        assert_eq!(path_without_extension.extension(), None);
    }

#[test]
    fn cloud_prefix_with_no_trailing_delimiter() {
        // Use case: searching with a prefix that matches a single file
        let prefix = Path::from("foo");
        assert_eq!(prefix.as_ref(), "foo");
    }

    #[test]
    fn multiple_encodes() {
        let location = Path::from_iter(["foo/bar", "baz/test%20file"]);
        assert_eq!(location.as_ref(), "foo%2Fbar/baz/test%20file");
    }

    #[test]
    fn parse_empty_segments() {
        // Edge case: parsing a path with multiple slashes
        let err = Path::parse("foo//bar").unwrap_err();
        assert!(matches!(err, Error::EmptySegment { .. }));
    }

    #[test]
    fn from_url_path_valid() {
        // Additional valid URL paths
        let a = Path::from_url_path("foo%20bar/test").unwrap();
        assert_eq!(a.raw, "foo bar/test");
    }

    #[test]
    fn from_url_path_invalid_unicode() {
        // Invalid Unicode segment should yield an error
        let err = Path::from_url_path("%80").unwrap_err();
        assert!(matches!(err, Error::NonUnicode { .. }));
    }

    #[test]
    fn filename_edge_cases() {
        // Edge case for filenames without extensions
        let a = Path::from("foo/bar/");
        let b = Path::from("foo/bar/baz/");
        
        assert_eq!(a.filename(), None);
        assert_eq!(b.filename(), None);
    }

    #[test]
    fn path_with_special_characters() {
        // Testing path containing special characters
        let a = Path::from("foo@bar#baz");
        assert_eq!(a.as_ref(), "foo@bar#baz");
    }

    #[test]
    fn path_containing_multiple_extensions() {
        // Path with multiple file extensions
        let a = Path::from("foo/bar.baz.qux");
        assert_eq!(a.extension(), Some("qux"));
    }

    #[test]
    fn prefix_matches_empty_path() {
        // Testing prefix matches with an empty path
        let haystack = Path::from("");
        let needle = Path::from("");

        assert!(haystack.prefix_matches(&needle));
    }

    #[test]
    fn prefix_matches_with_only_file_name() {
        // Prefix matching where the haystack is a file name only
        let haystack = Path::from("foo.json");
        let needle = Path::from("foo");

        assert!(haystack.prefix_matches(&needle));
    }

    #[test]
    fn prefix_matches_with_path_containing_spaces() {
        let haystack = Path::from("foo bar/baz");
        let needle = Path::from("foo bar");

        assert!(haystack.prefix_matches(&needle));
    }

    #[test]
    fn parts_after_prefix_with_empty_segments() {
        let existing_path = Path::from("apple//bear/cow/dog/egg.json");
        let prefix = Path::from("apple");

        let expected_parts: Vec<PathPart<'_>> = vec!["", "bear", "cow", "dog", "egg.json"]
            .into_iter()
            .map(Into::into)
            .collect();
        let parts: Vec<_> = existing_path.prefix_match(&prefix).unwrap().collect();
        assert_eq!(parts, expected_parts);
    }
}

