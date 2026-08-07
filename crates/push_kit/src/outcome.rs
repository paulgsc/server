//! What a send actually did.
//!
//! ## What `201` does not mean
//!
//! A push service returns `201 Created` once it has **accepted** a message for
//! delivery. That is all. Not delivered, not displayed, and — the trap — not
//! decryptable: a payload encrypted with the wrong keys returns `201` and is
//! then silently discarded by the browser, because the push service never had
//! the key material to notice. A `201` is not evidence that anything worked,
//! and nothing here reports it as though it were.
//!
//! The status codes that carry real information are the failures, which is why
//! each gets a distinct variant instead of being folded into `is_success()`.

/// The result of one delivery attempt to one endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
	/// `201`. Accepted for delivery, which is not delivery.
	Accepted,
	/// `404` or `410`. The subscription is gone; the caller should prune it.
	Expired,
	/// `403`. Signed with a key this subscription was not made with.
	KeyMismatch,
	/// `400`. A malformed request or a bad VAPID JWT — a bug on the sender's side.
	Rejected(String),
	/// `413`. Over the ceiling. Also caught before sending; this is the push
	/// service disagreeing about where the ceiling is.
	TooLarge,
	/// `429`. Carries `Retry-After` verbatim when the service sent one.
	RateLimited { retry_after: Option<String> },
	/// `5xx`, or anything unmapped.
	ServiceError { status: u16, body: String },
	/// The request never got a response.
	Transport(String),
	/// Encryption or signing failed before anything was sent.
	NotSent(String),
}

impl SendOutcome {
	/// Whether the subscription should be removed.
	///
	/// Only `404`/`410` says the browser is gone. A `403` is a **sender**
	/// misconfiguration — the wrong VAPID key — and pruning on it would destroy
	/// every subscription in one pass over one bad environment variable.
	#[must_use]
	pub const fn should_prune(&self) -> bool {
		matches!(self, Self::Expired)
	}

	/// Whether this counts against the subscription's failure tally.
	#[must_use]
	pub const fn is_failure(&self) -> bool {
		!matches!(self, Self::Accepted)
	}

	/// A stable label for logs and metrics. Not `Display`, which is free to
	/// carry detail; this is the low-cardinality dimension.
	#[must_use]
	pub const fn label(&self) -> &'static str {
		match self {
			Self::Accepted => "accepted",
			Self::Expired => "expired",
			Self::KeyMismatch => "key-mismatch",
			Self::Rejected(_) => "rejected",
			Self::TooLarge => "too-large",
			Self::RateLimited { .. } => "rate-limited",
			Self::ServiceError { .. } => "service-error",
			Self::Transport(_) => "transport-error",
			Self::NotSent(_) => "not-sent",
		}
	}

	/// Map a push service's response.
	///
	/// Separate from the request so the mapping — the part with the interesting
	/// failure modes — is testable without a network.
	#[must_use]
	pub fn from_status(status: u16, retry_after: Option<String>, body: String) -> Self {
		match status {
			200 | 201 | 202 | 204 => Self::Accepted,
			400 => Self::Rejected(body),
			403 => Self::KeyMismatch,
			404 | 410 => Self::Expired,
			413 => Self::TooLarge,
			429 => Self::RateLimited { retry_after },
			_ => Self::ServiceError { status, body },
		}
	}
}

#[cfg(test)]
mod tests {
	use super::SendOutcome;

	#[test]
	fn each_status_maps_to_its_own_outcome() {
		assert_eq!(SendOutcome::from_status(201, None, String::new()), SendOutcome::Accepted);
		assert_eq!(SendOutcome::from_status(403, None, String::new()), SendOutcome::KeyMismatch);
		assert_eq!(SendOutcome::from_status(404, None, String::new()), SendOutcome::Expired);
		assert_eq!(SendOutcome::from_status(410, None, String::new()), SendOutcome::Expired);
		assert_eq!(SendOutcome::from_status(413, None, String::new()), SendOutcome::TooLarge);
		assert!(matches!(SendOutcome::from_status(400, None, "bad".to_owned()), SendOutcome::Rejected(_)));
		assert!(matches!(SendOutcome::from_status(503, None, String::new()), SendOutcome::ServiceError { status: 503, .. }));
	}

	#[test]
	fn rate_limiting_carries_retry_after_through() {
		let outcome = SendOutcome::from_status(429, Some("120".to_owned()), String::new());
		assert_eq!(
			outcome,
			SendOutcome::RateLimited {
				retry_after: Some("120".to_owned())
			}
		);
	}

	#[test]
	fn only_a_dead_subscription_prunes() {
		assert!(SendOutcome::Expired.should_prune());
		assert!(!SendOutcome::KeyMismatch.should_prune());
		assert!(!SendOutcome::Accepted.should_prune());
		assert!(!SendOutcome::ServiceError { status: 503, body: String::new() }.should_prune());
	}

	#[test]
	fn acceptance_is_the_only_non_failure() {
		assert!(!SendOutcome::Accepted.is_failure());
		assert!(SendOutcome::Expired.is_failure());
		assert!(SendOutcome::Transport("down".to_owned()).is_failure());
	}
}
