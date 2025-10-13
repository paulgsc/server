use base64::engine::Engine;
use futures_util::{
	sink::SinkExt,
	stream::{SplitSink, SplitStream, StreamExt},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::error::Error;
use tokio::net::TcpStream;
use tokio_tungstenite::{tungstenite::protocol::Message as TungsteniteMessage, MaybeTlsStream};
use tracing::{info, warn};

pub async fn authenticate(
	password: &str,
	sink: &mut SplitSink<tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>,
	stream: &mut SplitStream<tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
	// Check if authentication is required
	let hello = wait_for_hello(stream).await?;
	let authentication = hello.get("d").and_then(|d| d.get("authentication")).and_then(Value::as_object);

	// If no authentication is required, just return
	if authentication.is_none() {
		warn!("No authentication required");
		maybe_identify(&hello, sink, stream).await?;
		return Ok(());
	}

	let authentication = authentication.ok_or("Missing authentication value")?;

	// Handle OBS 5.0+ authentication
	if let (Some(challenge), Some(salt)) = (authentication.get("challenge").and_then(Value::as_str), authentication.get("salt").and_then(Value::as_str)) {
		// Generate auth hash according to OBS WebSocket protocol
		let mut hasher = Sha256::new();
		hasher.update(password.as_bytes());
		hasher.update(salt.as_bytes());
		let first_hash = hasher.finalize();

		let mut second_hasher = Sha256::new();
		second_hasher.update(&first_hash[..]);
		second_hasher.update(challenge.as_bytes());
		let final_hash = second_hasher.finalize();

		let auth = base64::engine::general_purpose::STANDARD.encode(final_hash);

		// Send auth message
		let auth_msg = json!({
			"op": 1, // "Identify" op code
			"d": {
				"rpcVersion": 1,
				"authentication": auth,
				"eventSubscriptions": 33 // Subscribe to all events
			}
		});

		sink.send(TungsteniteMessage::Text(auth_msg.to_string().into())).await?;

		// Wait for authentication response
		let auth_response = match stream.next().await {
			Some(Ok(msg)) => msg,
			Some(Err(e)) => return Err(format!("WebSocket error: {e}").into()),
			None => return Err("WebSocket closed unexpectedly".into()),
		};

		// Parse the authentication response
		if let TungsteniteMessage::Text(text) = auth_response {
			let response: Value = serde_json::from_str(&text)?;
			let op = response.get("op").and_then(Value::as_u64);

			if op != Some(2) {
				// "Identified" op code
				return Err(format!("Authentication failed: {response:?}").into());
			}

			info!("Successfully authenticated with OBS WebSocket");
			Ok(())
		} else {
			Err("Expected text message for auth response".into())
		}
	} else {
		Err("Missing challenge or salt in authentication info".into())
	}
}

pub async fn maybe_identify(
	hello: &Value,
	sink: &mut SplitSink<tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>,
	stream: &mut SplitStream<tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
	if !hello.get("d").and_then(|d| d.get("authentication")).and_then(Value::as_object).is_some() {
		let identify_msg = json!({
			"op": 1,
			"d": {
				"rpcVersion": 1,
				"eventSubscriptions": 33
			}
		});

		sink.send(TungsteniteMessage::Text(identify_msg.to_string().into())).await?;

		let identify_response = match stream.next().await {
			Some(Ok(msg)) => msg,
			Some(Err(e)) => return Err(format!("WebSocket error: {e}").into()),
			None => return Err("WebSocket closed unexpectedly".into()),
		};

		let text = match identify_response {
			TungsteniteMessage::Text(t) => t,
			_ => return Err("Expected text message for identify response".into()),
		};

		let response: Value = serde_json::from_str(&text)?;
		let op = response.get("op").and_then(Value::as_u64);

		if op != Some(2) {
			return Err(format!("Identification failed: {response:?}").into());
		}

		info!("Successfully identified with OBS WebSocket");
	}

	Ok(())
}

async fn wait_for_hello(stream: &mut SplitStream<tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>>) -> Result<Value, Box<dyn Error + Send + Sync>> {
	while let Some(msg) = stream.next().await {
		match msg? {
			TungsteniteMessage::Text(text) => {
				let json: Value = serde_json::from_str(&text)?;

				if json.get("op").and_then(Value::as_u64) == Some(0) {
					return Ok(json);
				}
			}
			_ => {}
		}
	}

	Err("Connection closed before hello".into())
}
