use crate::auth::{AuthConfig, AuthConfigBuilder, AuthManager, Authenticated, Failed, Unauthenticated};
use crate::auth::{AuthTransport, AuthTransportError};
use crate::transport::{Connected, MessageContent, MessagePriority, OutgoingMessage, TransportError, TransportMessage, WebSocketTransport};
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

#[async_trait]
impl AuthTransport for WebSocketTransport<Connected> {
	async fn send_message(&mut self, message: Value) -> Result<(), AuthTransportError> {
		let json_string = serde_json::to_string(&message).map_err(|e| AuthTransportError::SendFailed {
			reason: format!("JSON serialization failed: {}", e),
		})?;

		let outgoing_message = OutgoingMessage {
			content: MessageContent::Text(json_string),
			priority: MessagePriority::High, // Auth messages are high priority
			timeout: Some(Duration::from_secs(10)),
			correlation_id: None,
			response_channel: None,
		};

		self.send_message(outgoing_message).await.map_err(|e| AuthTransportError::SendFailed {
			reason: format!("Transport send failed: {}", e),
		})?;

		Ok(())
	}

	async fn receive_message(&mut self, timeout: Duration) -> Result<Value, AuthTransportError> {
		// Subscribe to incoming messages
		let mut message_rx = self.subscribe_messages();

		// Wait for the next message with timeout
		match tokio::time::timeout(timeout, message_rx.recv()).await {
			Ok(Ok(transport_message)) => match transport_message {
				TransportMessage::Text { data, .. } => serde_json::from_str(&data).map_err(|e| AuthTransportError::ReceiveFailed {
					reason: format!("JSON parsing failed: {}", e),
				}),
				_ => Err(AuthTransportError::ReceiveFailed {
					reason: "Expected text message for auth".to_string(),
				}),
			},
			Ok(Err(_)) => Err(AuthTransportError::ReceiveFailed {
				reason: "Message channel closed".to_string(),
			}),
			Err(_) => Err(AuthTransportError::Timeout { duration: timeout }),
		}
	}
}

/// Enhanced WebSocket transport with integrated authentication
pub struct AuthenticatedWebSocketTransport {
	transport: Option<WebSocketTransport<Connected>>,
	auth_manager: Option<AuthManager<Authenticated>>,
}

impl AuthenticatedWebSocketTransport {
	/// Create a new authenticated transport
	pub fn new() -> Self {
		Self {
			transport: None,
			auth_manager: None,
		}
	}

	/// Connect and authenticate in one step
	pub async fn connect_and_authenticate(
		&mut self,
		endpoint: crate::transport::Endpoint,
		transport_config: crate::transport::TransportConfig,
		password: String,
	) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
		// First establish transport connection
		let disconnected_transport = WebSocketTransport::new(transport_config);
		let connecting_transport = disconnected_transport.connect(endpoint).await.map_err(|(_, e)| e)?;

		let mut connected_transport = connecting_transport.wait_for_connection().await.map_err(|(_, e)| e)?;

		// Wait for hello message directly on the transport
		let hello_message = connected_transport
			.receive_message(Duration::from_secs(10))
			.await
			.map_err(|e| format!("Failed to receive hello message: {}", e))?;

		// Create auth manager with the transport directly
		let auth_config = AuthConfigBuilder::new()
			.password(password)
			.timeout(Duration::from_secs(10))
			.build()
			.map_err(|e| format!("Invalid auth config: {}", e))?;

		let unauthenticated_auth = AuthManager::new(connected_transport);

		match unauthenticated_auth.authenticate(hello_message, auth_config).await {
			Ok(authenticated_auth) => {
				// Store the authenticated components
				self.auth_manager = Some(authenticated_auth);
				info!("Successfully authenticated with OBS WebSocket");
				Ok(())
			}
			Err((failed_auth, error)) => {
				error!("Authentication failed: {}", error);
				Err(Box::new(error))
			}
		}
	}

	/// Send a message (requires authentication)
	pub async fn send_message(&self, message: Value) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
		if let Some(ref auth_manager) = self.auth_manager {
			// In practice, you'd need a way to send messages through the authenticated manager
			// This would require extending the auth manager interface
			Ok(())
		} else {
			Err("Not authenticated".into())
		}
	}

	/// Check if authenticated
	pub fn is_authenticated(&self) -> bool {
		self.auth_manager.is_some()
	}

	/// Get session info
	pub fn session_id(&self) -> Option<crate::auth::SessionId> {
		self.auth_manager.as_ref().map(|auth| auth.session_info())
	}

	/// Reset authentication (for reconnection scenarios)
	pub async fn reset_auth(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
		if let Some(auth_manager) = self.auth_manager.take() {
			let unauthenticated = auth_manager.reset().await?;
			// Store the unauthenticated manager if needed for re-authentication
		}
		Ok(())
	}
}

/// Factory function to create an authenticated OBS WebSocket client
pub async fn create_authenticated_obs_client(host: String, port: u16, password: String) -> Result<AuthenticatedWebSocketTransport, Box<dyn std::error::Error + Send + Sync>> {
	let endpoint = crate::transport::Endpoint {
		host,
		port,
		path: "/".to_string(),
		use_tls: false,
	};

	let transport_config = crate::transport::TransportConfig::default();

	let mut client = AuthenticatedWebSocketTransport::new();
	client.connect_and_authenticate(endpoint, transport_config, password).await?;

	Ok(client)
}

// Default implementation for TransportConfig
impl Default for crate::transport::TransportConfig {
	fn default() -> Self {
		Self {
			connection: crate::transport::ConnectionConfig {
				connect_timeout: Duration::from_secs(10),
				handshake_timeout: Duration::from_secs(10),
				close_timeout: Duration::from_secs(5),
				max_frame_size: 64 * 1024,
				max_message_size: 1024 * 1024,
				subprotocols: vec!["obs-websocket".to_string()],
				custom_headers: std::collections::HashMap::new(),
			},
			tls: crate::transport::TlsConfig {
				enabled: false,
				verify_hostname: true,
				ca_certificates: None,
				client_certificate: None,
				cipher_suites: vec![],
				protocol_versions: vec![],
			},
			compression: crate::transport::CompressionConfig {
				enabled: false,
				algorithm: crate::transport::CompressionAlgorithm::None,
				window_bits: 15,
				compression_level: 6,
				threshold: 1024,
			},
			flow_control: crate::transport::FlowControlConfig {
				send_buffer_size: 1024,
				receive_buffer_size: 1024,
				backpressure_threshold: 800,
				max_pending_frames: 100,
				credit_based_flow_control: true,
			},
			timeouts: crate::transport::TimeoutConfig {
				ping_interval: Duration::from_secs(30),
				pong_timeout: Duration::from_secs(5),
				idle_timeout: Duration::from_secs(120),
				write_timeout: Duration::from_secs(10),
				read_timeout: Duration::from_secs(10),
			},
			buffer_sizes: crate::transport::BufferConfig {
				send_queue_size: 1000,
				receive_queue_size: 1000,
				frame_buffer_size: 64 * 1024,
				message_buffer_size: 1024 * 1024,
			},
			keepalive: crate::transport::KeepaliveConfig {
				enabled: true,
				interval: Duration::from_secs(30),
				timeout: Duration::from_secs(5),
				max_failures: 3,
			},
		}
	}
}
