//! A value that must never reach a log line.
//!
//! Session tokens and passkey credential ids are the secrets this server holds
//! (docs/identity.md, "Secrets are wrapped when they exist"). Wrapping one in
//! [`Redacted`] makes printing it a thing you cannot do by accident: its
//! `Debug` prints `[redacted]`, so a `?field` in a tracing macro or a `{:?}` of
//! a struct holding one leaks nothing, and it has no `Display`, so a
//! `%field` or a `{}` does not compile. The value is reached only through
//! [`Redacted::expose`], which is greppable.
//!
//! ```
//! let token = file_host::redacted::Redacted::new([7_u8; 4]);
//! assert_eq!(format!("{token:?}"), "[redacted]");
//! assert_eq!(token.expose(), &[7, 7, 7, 7]);
//! ```
//!
//! ```compile_fail
//! let token = file_host::redacted::Redacted::new(String::from("secret"));
//! let _ = format!("{token}");
//! ```

use std::fmt;

#[derive(Clone, PartialEq, Eq)]
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
	#[must_use]
	pub const fn new(value: T) -> Self {
		Self(value)
	}

	/// The wrapped value. Named so every read of a secret is findable.
	#[must_use]
	pub const fn expose(&self) -> &T {
		&self.0
	}
}

impl<T> fmt::Debug for Redacted<T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("[redacted]")
	}
}
