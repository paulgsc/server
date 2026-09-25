//! Request/response correlation for commands sent to OBS.
//!
//! OBS answers every `op: 6` request with an `op: 7` response that echoes its
//! `requestId`. Each command registers a reply channel under its id before it
//! is sent; the read loop resolves that channel when the matching response
//! arrives, so callers learn what OBS actually said instead of only that the
//! request left the socket.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::oneshot;

/// Why a command did not complete successfully.
#[derive(Debug, Clone, Error)]
pub enum CommandError {
	/// OBS received the request and refused it. `comment` is OBS's own reason,
	/// e.g. "The stream output is already running."
	#[error("OBS rejected {request_type} (code {code}): {comment}")]
	Rejected { request_type: String, code: u64, comment: String },

	#[error("Invalid request: {0}")]
	InvalidRequest(String),

	#[error("OBS did not respond within {0:?}")]
	Timeout(Duration),

	#[error("Connection to OBS closed before it responded")]
	ConnectionClosed,
}

/// OBS's `responseData` on success; `None` for requests that return nothing.
pub type CommandResult = Result<Option<Value>, CommandError>;
pub type CommandReply = oneshot::Sender<CommandResult>;

/// Reply channels for requests awaiting OBS's response, keyed by `requestId`.
///
/// Scoped to one connection: the read loop calls [`Self::fail_all`] when the
/// socket ends, so no caller waits on a response that can no longer arrive.
/// A caller that times out leaves its entry behind until OBS answers late or
/// the connection ends, whichever comes first.
#[derive(Clone, Default)]
pub struct PendingRequests {
	inner: Arc<Mutex<HashMap<String, CommandReply>>>,
}

impl PendingRequests {
	pub fn register(&self, request_id: String, reply: CommandReply) {
		self.lock().insert(request_id, reply);
	}

	/// Resolves the pending request `message` answers, if it is a response to
	/// one. Everything else, including responses to polling requests, is
	/// ignored.
	pub fn resolve(&self, message: &Value) {
		if message.get("op").and_then(Value::as_u64) != Some(7) {
			return;
		}
		let Some(d) = message.get("d") else { return };
		let Some(request_id) = d.get("requestId").and_then(Value::as_str) else {
			return;
		};
		let Some(reply) = self.lock().remove(request_id) else {
			return;
		};

		// The caller may have timed out and dropped its receiver; nothing to do then.
		let _ = reply.send(response_result(d));
	}

	/// Fails every outstanding request; called when the connection ends.
	pub fn fail_all(&self) {
		for (_, reply) in self.lock().drain() {
			let _ = reply.send(Err(CommandError::ConnectionClosed));
		}
	}

	fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CommandReply>> {
		// The map holds no invariant a panicking holder could break.
		self.inner.lock().unwrap_or_else(PoisonError::into_inner)
	}
}

fn response_result(d: &Value) -> CommandResult {
	let status = d.get("requestStatus");
	let succeeded = status.and_then(|s| s.get("result")).and_then(Value::as_bool).unwrap_or(false);

	if succeeded {
		return Ok(d.get("responseData").cloned());
	}

	Err(CommandError::Rejected {
		request_type: d.get("requestType").and_then(Value::as_str).unwrap_or("unknown").to_owned(),
		code: status.and_then(|s| s.get("code")).and_then(Value::as_u64).unwrap_or(0),
		comment: status
			.and_then(|s| s.get("comment"))
			.and_then(Value::as_str)
			.unwrap_or("no comment from OBS")
			.to_owned(),
	})
}

/// Returns the request's `requestId`, generating one for an `op: 6` request
/// that lacks it. `None` means the message is not a single request and so gets
/// no response OBS would echo an id on.
pub fn ensure_request_id(request: &mut Value) -> Option<String> {
	if request.get("op").and_then(Value::as_u64) != Some(6) {
		return None;
	}
	let d = request.get_mut("d")?.as_object_mut()?;
	if let Some(id) = d.get("requestId").and_then(Value::as_str) {
		return Some(id.to_owned());
	}
	let id = uuid::Uuid::new_v4().simple().to_string();
	d.insert("requestId".to_owned(), Value::String(id.clone()));
	Some(id)
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn pending_with(id: &str) -> (PendingRequests, oneshot::Receiver<CommandResult>) {
		let pending = PendingRequests::default();
		let (tx, rx) = oneshot::channel();
		pending.register(id.to_owned(), tx);
		(pending, rx)
	}

	#[tokio::test]
	async fn success_resolves_with_response_data() {
		let (pending, rx) = pending_with("req-1");
		pending.resolve(&json!({
			"op": 7,
			"d": {
				"requestType": "GetInputMute",
				"requestId": "req-1",
				"requestStatus": { "result": true, "code": 100 },
				"responseData": { "inputMuted": true }
			}
		}));

		let data = rx.await.unwrap().unwrap();
		assert_eq!(data, Some(json!({ "inputMuted": true })));
	}

	#[tokio::test]
	async fn rejection_carries_obs_code_and_comment() {
		let (pending, rx) = pending_with("req-2");
		pending.resolve(&json!({
			"op": 7,
			"d": {
				"requestType": "StartStream",
				"requestId": "req-2",
				"requestStatus": { "result": false, "code": 500, "comment": "The stream output is already running." }
			}
		}));

		match rx.await.unwrap() {
			Err(CommandError::Rejected { request_type, code, comment }) => {
				assert_eq!(request_type, "StartStream");
				assert_eq!(code, 500);
				assert_eq!(comment, "The stream output is already running.");
			}
			other => panic!("expected rejection, got {other:?}"),
		}
	}

	#[tokio::test]
	async fn unrelated_messages_leave_the_request_pending() {
		let (pending, mut rx) = pending_with("req-3");
		pending.resolve(&json!({ "op": 7, "d": { "requestId": "someone-else", "requestStatus": { "result": true } } }));
		pending.resolve(&json!({ "op": 5, "d": { "eventType": "StreamStateChanged", "requestId": "req-3" } }));

		assert!(rx.try_recv().is_err());
		pending.fail_all();
		assert!(matches!(rx.await.unwrap(), Err(CommandError::ConnectionClosed)));
	}

	#[test]
	fn ensure_request_id_keeps_or_fills_ids_on_single_requests_only() {
		let mut with_id = json!({ "op": 6, "d": { "requestType": "GetVersion", "requestId": "mine" } });
		assert_eq!(ensure_request_id(&mut with_id).as_deref(), Some("mine"));

		let mut without_id = json!({ "op": 6, "d": { "requestType": "GetVersion" } });
		let generated = ensure_request_id(&mut without_id).unwrap();
		assert_eq!(without_id["d"]["requestId"], json!(generated));

		let mut batch = json!({ "op": 8, "d": { "requestId": "batch", "requests": [] } });
		assert_eq!(ensure_request_id(&mut batch), None);
	}
}
