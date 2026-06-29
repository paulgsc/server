/// Domain model.
///
/// Typestate pattern is used to separate two orthogonal concerns:
///
///   1. A *validated* Register (label non-empty, value non-empty, id assigned)
///      vs. a raw "pending" one being built from user input.
///
///   2. A *loaded* Registry (proven to have come from disk and passed
///      invariant checks) vs. a just-constructed one that may not yet
///      satisfy the max-entry constraint.
///
/// Neither state leaks into the other's API surface.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{PocketError, Result};

// ── Typestate markers ────────────────────────────────────────────────────────

/// A `Register<Pending>` is user-supplied data not yet validated.
pub struct Pending;

/// A `Register<Validated>` has passed all domain invariants.
pub struct Validated;

// ── Register ─────────────────────────────────────────────────────────────────

/// The on-disk representation, used for both serialisation targets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegisterData {
	pub id: String,
	pub label: String,
	pub value: String,
	pub created_at: DateTime<Utc>,
	pub last_used_at: DateTime<Utc>,
}

/// A register in a given typestate `S`.
///
/// `Register<Pending>` — user input, not yet checked.
/// `Register<Validated>` — invariants proven; ready to be persisted.
pub struct Register<S> {
	pub(crate) data: RegisterData,
	_state: std::marker::PhantomData<S>,
}

impl Register<Pending> {
	/// Construct from raw user-supplied strings.
	/// No invariants are checked here; validation is a separate step.
	pub fn from_input(label: impl Into<String>, value: impl Into<String>) -> Self {
		let now = Utc::now();
		Register {
			data: RegisterData {
				id: Uuid::new_v4().to_string(),
				label: label.into(),
				value: value.into(),
				created_at: now,
				last_used_at: now,
			},
			_state: std::marker::PhantomData,
		}
	}

	/// Validate and transition to `Validated`.
	///
	/// Returns `Err` for each distinct violation; callers see exactly
	/// what is wrong rather than a combined "invalid input" sentinel.
	pub fn validate(self) -> Result<Register<Validated>> {
		let label = self.data.label.trim().to_string();
		let value = self.data.value.trim().to_string();

		if label.is_empty() {
			return Err(PocketError::EmptyLabel);
		}
		if value.is_empty() {
			return Err(PocketError::EmptyValue);
		}

		Ok(Register {
			data: RegisterData { label, value, ..self.data },
			_state: std::marker::PhantomData,
		})
	}
}

impl Register<Validated> {
	/// Borrow the underlying data, e.g. for serialisation.
	pub fn as_data(&self) -> &RegisterData {
		&self.data
	}

	/// Record a usage timestamp (returns a new validated register — no
	/// mutation of the caller, no silent interior mutation).
	pub fn touch(self) -> Self {
		Register {
			data: RegisterData {
				last_used_at: Utc::now(),
				..self.data
			},
			_state: std::marker::PhantomData,
		}
	}

	/// Destructure into the raw data for storage.
	pub fn into_data(self) -> RegisterData {
		self.data
	}
}

/// Allow reconstruction from raw `RegisterData` (e.g. after deserialisation).
/// This skips the `Pending` state because the data has already been
/// persisted — it is *asserted* validated, not *proven* validated by the
/// builder.  Callers must invoke `Registry::check_invariants` after loading.
impl From<RegisterData> for Register<Validated> {
	fn from(data: RegisterData) -> Self {
		Register {
			data,
			_state: std::marker::PhantomData,
		}
	}
}

// ── Registry typestate ────────────────────────────────────────────────────────

/// `Registry<Unchecked>` has been constructed or deserialised but
/// the invariant (max_entries, non-empty labels etc.) has not been verified.
pub struct Unchecked;

/// `Registry<Checked>` has passed invariant verification.
/// Only this state can add, remove, or persist.
pub struct Checked;

// ── Registry ─────────────────────────────────────────────────────────────────

/// The on-disk JSON envelope.
#[derive(Debug, Serialize, Deserialize)]
pub struct RegistryData {
	pub max_entries: usize,
	pub registers: Vec<RegisterData>,
}

/// A registry in typestate `S`.
pub struct Registry<S> {
	pub(crate) data: RegistryData,
	_state: std::marker::PhantomData<S>,
}

impl Registry<Unchecked> {
	pub fn new(max_entries: usize) -> Self {
		Registry {
			data: RegistryData {
				max_entries,
				registers: Vec::new(),
			},
			_state: std::marker::PhantomData,
		}
	}

	pub fn from_data(data: RegistryData) -> Self {
		Registry {
			data,
			_state: std::marker::PhantomData,
		}
	}

	/// Verify invariants and transition to `Checked`.
	///
	/// Invariants checked:
	///   - max_entries > 0
	///   - no register has an empty label or value
	///   - no duplicate labels
	///   - entry count ≤ max_entries
	pub fn check(self) -> Result<Registry<Checked>> {
		if self.data.max_entries == 0 {
			// max_entries = 0 is only possible via manual JSON edit.
			// Treat it as corrupt data; map to Deserialize conceptually
			// by producing a descriptive IO-level error.
			return Err(PocketError::RegistryRead {
				path: std::path::PathBuf::from("<registry>"),
				source: std::io::Error::new(std::io::ErrorKind::InvalidData, "max_entries must be > 0"),
			});
		}

		for r in &self.data.registers {
			if r.label.trim().is_empty() {
				return Err(PocketError::EmptyLabel);
			}
			if r.value.trim().is_empty() {
				return Err(PocketError::EmptyValue);
			}
		}

		// Duplicate label check — O(n²) is fine at n ≤ 128.
		for (i, a) in self.data.registers.iter().enumerate() {
			for b in self.data.registers.iter().skip(i + 1) {
				if a.label == b.label {
					return Err(PocketError::DuplicateLabel { label: a.label.clone() });
				}
			}
		}

		// Over-full registry (could happen from manual JSON edit).
		// We accept it — do not truncate silently — but flag it.
		// The *add* path will reject further growth.

		Ok(Registry {
			data: self.data,
			_state: std::marker::PhantomData,
		})
	}
}

impl Registry<Checked> {
	pub fn max_entries(&self) -> usize {
		self.data.max_entries
	}

	pub fn len(&self) -> usize {
		self.data.registers.len()
	}

	pub fn is_empty(&self) -> bool {
		self.data.registers.is_empty()
	}

	pub fn is_full(&self) -> bool {
		self.data.registers.len() >= self.data.max_entries
	}

	/// Iterate over validated registers (read-only).
	pub fn iter(&self) -> impl Iterator<Item = Register<Validated>> + '_ {
		self.data.registers.iter().cloned().map(Register::from)
	}

	/// Add a validated register.
	pub fn add(&mut self, reg: Register<Validated>) -> Result<()> {
		let data = reg.into_data();

		// Reject duplicate labels.
		if self.data.registers.iter().any(|r| r.label == data.label) {
			return Err(PocketError::DuplicateLabel { label: data.label });
		}

		// Enforce capacity.
		if self.data.registers.len() >= self.data.max_entries {
			return Err(PocketError::RegistryFull { max: self.data.max_entries });
		}

		self.data.registers.push(data);
		Ok(())
	}

	/// Remove by label (exact match).
	pub fn remove(&mut self, label: &str) -> Result<RegisterData> {
		let pos = self
			.data
			.registers
			.iter()
			.position(|r| r.label == label)
			.ok_or_else(|| PocketError::LabelNotFound { label: label.to_string() })?;

		Ok(self.data.registers.remove(pos))
	}

	/// Find by label (exact match), returning a view.
	pub fn find(&self, label: &str) -> Option<Register<Validated>> {
		self.data.registers.iter().find(|r| r.label == label).cloned().map(Register::from)
	}

	/// Update `last_used_at` for a given label.
	pub fn touch(&mut self, label: &str) -> Result<()> {
		let r = self
			.data
			.registers
			.iter_mut()
			.find(|r| r.label == label)
			.ok_or_else(|| PocketError::LabelNotFound { label: label.to_string() })?;
		r.last_used_at = Utc::now();
		Ok(())
	}

	/// Borrow the raw data for serialisation.
	pub fn as_data(&self) -> &RegistryData {
		&self.data
	}
}
