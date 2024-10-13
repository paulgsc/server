# Path and PathPart Modules

## Overview

This Rust library provides robust implementations for handling file system and URL paths through two main modules: `Path` and `PathPart`. These modules offer a unified and flexible approach to path manipulation, parsing, and representation, with a focus on maintaining the original path structure while providing useful utilities for path operations.

## Table of Contents

1. [Path Module](#path-module)
2. [PathPart Module](#pathpart-module)
3. [Usage Examples](#usage-examples)
4. [Error Handling](#error-handling)
5. [Security Considerations](#security-considerations)
6. [License](#license)

## Path Module

The `Path` module provides a high-level abstraction for working with file system and URL paths.

### Key Features

- Unified representation for both file system and URL paths
- Parsing and manipulation of path components
- URL decoding support
- Prefix matching capabilities
- Immutable design for thread-safety

### Main Struct: `Path`

The `Path` struct is the core of this module, representing a path as a series of segments.

#### Important Methods

- `parse(s: &str) -> Result<Self, PathPartError>`: Parses a string into a `Path`.
- `from_url_path(path: impl AsRef<str>) -> Result<Self, PathPartError>`: Creates a `Path` from a URL-encoded string.
- `prefix_match(&self, prefix: &Path) -> Option<impl Iterator<Item = PathPart<'_>>>`: Checks if the path starts with a given prefix.
- `parts(&self) -> impl Iterator<Item = PathPart<'_>>`: Returns an iterator over the path segments.
- `filename(&self) -> Option<&str>`: Returns the last segment of the path.
- `extension(&self) -> Option<&str>`: Returns the file extension, if any.
- `child(&self, segment: impl Into<PathPart<'_>>) -> Self`: Creates a new path by appending a segment.

### Implementations

The `Path` struct implements several traits for flexible creation and manipulation:

- `Default`
- `FromIterator`
- `From<&str>`
- `From<String>`
- `Into<String>`
- `Display`

## PathPart Module

The `PathPart` module deals with individual segments of a path.

### Key Features

- Validation of individual path segments
- Handling of special characters and URL encoding
- Conversion between different representations of path segments

### Main Struct: `PathPart`

The `PathPart` struct represents a single segment of a path.

#### Important Methods

- `parse(s: &str) -> Result<Self, PathPartError>`: Parses a string into a `PathPart`.
- `as_str(&self) -> &str`: Returns the raw string representation of the path part.

### Error Handling

The module uses a custom `PathPartError` enum to provide detailed error information for various failure scenarios.

## Usage Examples

Here are some examples demonstrating the usage of the `Path` and `PathPart` modules:

```rust
use path::{Path, PathPart};

// Creating a path
let path = Path::parse("/foo/bar/baz.txt").unwrap();

// Getting the filename
assert_eq!(path.filename(), Some("baz.txt"));

// Getting the extension
assert_eq!(path.extension(), Some("txt"));

// Creating a child path
let child_path = path.child("qux");
assert_eq!(child_path.as_ref(), "foo/bar/baz.txt/qux");

// Parsing a URL-encoded path
let url_path = Path::from_url_path("foo%20bar/baz%20qux").unwrap();
assert_eq!(url_path.as_ref(), "foo bar/baz qux");

// Prefix matching
let prefix = Path::parse("/foo/bar").unwrap();
assert!(path.prefix_matches(&prefix));
```

## Error Handling

Both modules use a custom `PathPartError` enum for error handling. This enum provides detailed information about parsing failures and other error conditions. Always check the `Result` returned by parsing functions to ensure proper error handling in your application.

## Security Considerations

While these modules provide robust path handling, they do not perform security-related path normalization or validation. It's crucial to implement additional security measures when using these paths to access actual file systems or resources. Consider the following:

- Implement path normalization to prevent directory traversal attacks.
- Use a whitelist of allowed directories or resources.
- Employ sandboxing techniques when necessary.
- Always validate and sanitize user input before creating `Path` or `PathPart` instances.

## License

This library is licensed under the Apache License, Version 2.0. See the LICENSE file for more details.

---

For more detailed information about specific functions and their usage, please refer to the inline documentation in the source code.
