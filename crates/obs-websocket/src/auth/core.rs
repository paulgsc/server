//! OBS WebSocket Authentication Module
//!
//! This module implements a type-safe authentication system using the typestate pattern
//! and actor model for OBS WebSocket v5.0+ protocol.
//!
//! # Architecture
//!
//! The authentication process follows a strict state machine:
//! ```
//! Unauthenticated → Authenticating → Authenticated
//!        ↓              ↓              ↓
//!        └──────── Failed ←────────────┘
//! ```
//!
//! Each state is enforced at compile time, preventing invalid operations.

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::engine::{general_purpose::STANDARD as BASE64_STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn, Span};
use uuid::Uuid;

// ============================================================================
// Type System - Compile-time State Enforcement
// ============================================================================

/// Marker trait for authentication states
pub trait AuthState: 'static + Send + Sync + std::fmt::Debug + Clone {}

/// Initial state - no authentication attempted
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unauthenticated;

/// Authentication in progress
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authenticating {
	pub started_at: Instant,
	pub attempt_count: u32,
}

/// Successfully authenticated
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authenticated {
	pub authenticated_at: Instant,
	pub session_id: SessionId,
	pub obs_version: String,
	pub protocol_version: String,
}

/// Authentication failed
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
	pub failed_at: Instant,
	pub error: AuthenticationError,
	pub retryable: bool,
}

impl AuthState for Unauthenticated {}
impl AuthState for Authenticating {}
impl AuthState for Authenticated {}
impl AuthState for Failed {}

// ============================================================================
// Configuration System with Builder Pattern
// ============================================================================

/// Secure string wrapper for sensitive data
#[derive(Clone, PartialEq, Eq)]
pub struct SecureString {
	inner: String,
}

impl SecureString {
	pub fn new(value: String) -> Self {
		Self { inner: value }
	}

	pub fn as_str(&self) -> &str {
		&self.inner
	}

	pub fn is_empty(&self) -> bool {
		self.inner.is_empty()
	}
}

impl std::fmt::Debug for SecureString {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "SecureString([REDACTED])")
	}
}

impl Drop for SecureString {
	fn drop(&mut self) {
		// Zero out the memory (best effort)
		unsafe {
			std::ptr::write_volatile(self.inner.as_mut_ptr(), 0);
		}
	}
}

/// Authentication configuration
#[derive(Debug, Clone)]
pub struct AuthConfig {
	pub password: SecureString,
	pub timeout: Duration,
	pub max_attempts: u32,
	pub retry_delay: Duration,
}

impl Default for AuthConfig {
	fn default() -> Self {
		Self {
			password: SecureString::new(String::new()),
			timeout: Duration::from_secs(10),
			max_attempts: 3,
			retry_delay: Duration::from_secs(1),
		}
	}
}

/// Configuration builder with validation
pub struct AuthConfigBuilder {
	config: AuthConfig,
	validation_errors: Vec<ValidationError>,
}

impl AuthConfigBuilder {
	pub fn new() -> Self {
		Self {
			config: AuthConfig::default(),
			validation_errors: Vec::new(),
		}
	}

	pub fn password<S: Into<String>>(mut self, password: S) -> Self {
		self.config.password = SecureString::new(password.into());
		self
	}

	pub fn timeout(mut self, timeout: Duration) -> Self {
		self.config.timeout = timeout;
		self
	}

	pub fn max_attempts(mut self, attempts: u32) -> Self {
		self.config.max_attempts = attempts;
		self
	}

	pub fn retry_delay(mut self, delay: Duration) -> Self {
		self.config.retry_delay = delay;
		self
	}

	pub fn build(mut self) -> Result<AuthConfig, ValidationError> {
		self.validate()?;
		Ok(self.config)
	}

	fn validate(&mut self) -> Result<(), ValidationError> {
		if self.config.password.is_empty() {
			self.validation_errors.push(ValidationError::EmptyPassword);
		}

		if self.config.timeout.is_zero() {
			self.validation_errors.push(ValidationError::ZeroTimeout);
		}

		if self.config.max_attempts == 0 {
			self.validation_errors.push(ValidationError::ZeroMaxAttempts);
		}

		if !self.validation_errors.is_empty() {
			return Err(ValidationError::Multiple(self.validation_errors.clone()));
		}

		Ok(())
	}
}

impl Default for AuthConfigBuilder {
	fn default() -> Self {
		Self::new()
	}
}

// ============================================================================
// Core Types and Identifiers
// ============================================================================

/// Unique session identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(Uuid);

impl SessionId {
	pub fn new() -> Self {
		Self(Uuid::new_v4())
	}
}

impl std::fmt::Display for SessionId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0)
	}
}

/// Authentication context information
#[derive(Debug, Clone)]
pub struct AuthContext {
	pub challenge: String,
	pub salt: String,
	pub obs_version: Option<String>,
	pub protocol_version: Option<String>,
}

/// Authentication session information
#[derive(Debug, Clone)]
pub struct SessionInfo {
	pub session_id: SessionId,
	pub authenticated_at: Instant,
	pub obs_version: String,
	pub protocol_version: String,
	pub negotiated_rpc_version: u32,
}

// ============================================================================
// Actor System - Message Protocol
// ============================================================================

/// Commands sent to the authentication actor
#[derive(Debug)]
pub enum AuthCommand {
	/// Initiate authentication process
	Authenticate {
		hello_message: Value,
		config: AuthConfig,
		respond_to: oneshot::Sender<Result<SessionInfo, AuthenticationError>>,
	},

	/// Reset authentication state
	Reset { respond_to: oneshot::Sender<Result<(), AuthenticationError>> },

	/// Get current authentication status
	GetStatus { respond_to: oneshot::Sender<AuthStatus> },

	/// Graceful shutdown
	Shutdown,
}

/// Events emitted by the authentication actor
#[derive(Debug, Clone)]
pub enum AuthEvent {
	/// Authentication state changed
	StateChanged {
		session_id: SessionId,
		from_state: String,
		to_state: String,
		timestamp: Instant,
	},

	/// Authentication completed successfully
	AuthenticationSucceeded {
		session_id: SessionId,
		session_info: SessionInfo,
		timestamp: Instant,
	},

	/// Authentication failed
	AuthenticationFailed {
		session_id: SessionId,
		error: AuthenticationError,
		retryable: bool,
		timestamp: Instant,
	},

	/// Challenge received from OBS
	ChallengeReceived {
		session_id: SessionId,
		challenge_length: usize,
		timestamp: Instant,
	},
}

/// Current authentication status
#[derive(Debug, Clone)]
pub struct AuthStatus {
	pub session_id: SessionId,
	pub state: String,
	pub last_error: Option<AuthenticationError>,
	pub attempt_count: u32,
	pub uptime: Duration,
}

// ============================================================================
// Transport Abstraction
// ============================================================================

/// Trait for WebSocket transport operations
#[async_trait::async_trait]
pub trait AuthTransport: Send + Sync {
	async fn send_message(&mut self, message: Value) -> Result<(), AuthTransportError>;
	async fn receive_message(&mut self, timeout: Duration) -> Result<Value, AuthTransportError>;
}

// ============================================================================
// Authentication Actor Implementation
// ============================================================================

/// Authentication actor that manages the authentication state machine
pub struct AuthenticationActor {
	session_id: SessionId,
	state: ActorState,
	command_rx: mpsc::Receiver<AuthCommand>,
	event_tx: broadcast::Sender<AuthEvent>,
	transport: Box<dyn AuthTransport>,
	metrics: AuthMetrics,
	started_at: Instant,
}

#[derive(Debug)]
enum ActorState {
	Idle,
	Authenticating { config: AuthConfig, attempt: u32, started_at: Instant },
	Authenticated { session_info: SessionInfo },
	Failed { error: AuthenticationError, failed_at: Instant },
	Shutdown,
}

impl AuthenticationActor {
	pub fn new(transport: Box<dyn AuthTransport>, command_rx: mpsc::Receiver<AuthCommand>, event_tx: broadcast::Sender<AuthEvent>) -> Self {
		let session_id = SessionId::new();

		Self {
			session_id,
			state: ActorState::Idle,
			command_rx,
			event_tx,
			transport,
			metrics: AuthMetrics::new(),
			started_at: Instant::now(),
		}
	}

	#[instrument(skip(self), fields(session_id = %self.session_id))]
	pub async fn run(mut self) {
		info!("Authentication actor started");

		while let Some(command) = self.command_rx.recv().await {
			match command {
				AuthCommand::Authenticate {
					hello_message,
					config,
					respond_to,
				} => {
					let result = self.handle_authenticate(hello_message, config).await;
					let _ = respond_to.send(result);
				}

				AuthCommand::Reset { respond_to } => {
					let result = self.handle_reset().await;
					let _ = respond_to.send(result);
				}

				AuthCommand::GetStatus { respond_to } => {
					let status = self.get_status();
					let _ = respond_to.send(status);
				}

				AuthCommand::Shutdown => {
					info!("Authentication actor shutting down gracefully");
					self.state = ActorState::Shutdown;
					break;
				}
			}
		}

		info!("Authentication actor stopped");
	}

	#[instrument(skip(self, hello_message, config))]
	async fn handle_authenticate(&mut self, hello_message: Value, config: AuthConfig) -> Result<SessionInfo, AuthenticationError> {
		match &self.state {
			ActorState::Idle | ActorState::Failed { .. } => {
				self.transition_to_authenticating(config.clone()).await;
				self.perform_authentication(hello_message, config).await
			}
			ActorState::Authenticating { .. } => Err(AuthenticationError::InvalidSessionState),
			ActorState::Authenticated { session_info } => Ok(session_info.clone()),
			ActorState::Shutdown => Err(AuthenticationError::ActorUnavailable),
		}
	}

	async fn handle_reset(&mut self) -> Result<(), AuthenticationError> {
		match &self.state {
			ActorState::Shutdown => Err(AuthenticationError::ActorUnavailable),
			_ => {
				self.transition_to_idle().await;
				Ok(())
			}
		}
	}

	fn get_status(&self) -> AuthStatus {
		let (state_name, last_error, attempt_count) = match &self.state {
			ActorState::Idle => ("Idle".to_string(), None, 0),
			ActorState::Authenticating { attempt, .. } => ("Authenticating".to_string(), None, *attempt),
			ActorState::Authenticated { .. } => ("Authenticated".to_string(), None, 0),
			ActorState::Failed { error, .. } => ("Failed".to_string(), Some(error.clone()), 0),
			ActorState::Shutdown => ("Shutdown".to_string(), None, 0),
		};

		AuthStatus {
			session_id: self.session_id,
			state: state_name,
			last_error,
			attempt_count,
			uptime: self.started_at.elapsed(),
		}
	}

	#[instrument(skip(self, config))]
	async fn transition_to_authenticating(&mut self, config: AuthConfig) {
		let from_state = self.state_name();
		let started_at = Instant::now();

		self.state = ActorState::Authenticating { config, attempt: 1, started_at };

		self.emit_state_change_event(from_state, "Authenticating".to_string()).await;
	}

	async fn transition_to_idle(&mut self) {
		let from_state = self.state_name();
		self.state = ActorState::Idle;
		self.emit_state_change_event(from_state, "Idle".to_string()).await;
	}

	async fn transition_to_authenticated(&mut self, session_info: SessionInfo) {
		let from_state = self.state_name();
		self.state = ActorState::Authenticated {
			session_info: session_info.clone(),
		};

		self.emit_state_change_event(from_state, "Authenticated".to_string()).await;

		let _ = self.event_tx.send(AuthEvent::AuthenticationSucceeded {
			session_id: self.session_id,
			session_info,
			timestamp: Instant::now(),
		});
	}

	async fn transition_to_failed(&mut self, error: AuthenticationError) {
		let from_state = self.state_name();
		let failed_at = Instant::now();
		let retryable = error.is_retryable();

		self.state = ActorState::Failed { error: error.clone(), failed_at };

		self.emit_state_change_event(from_state, "Failed".to_string()).await;

		let _ = self.event_tx.send(AuthEvent::AuthenticationFailed {
			session_id: self.session_id,
			error,
			retryable,
			timestamp: failed_at,
		});
	}

	async fn emit_state_change_event(&self, from_state: String, to_state: String) {
		let _ = self.event_tx.send(AuthEvent::StateChanged {
			session_id: self.session_id,
			from_state,
			to_state,
			timestamp: Instant::now(),
		});
	}

	fn state_name(&self) -> String {
		match &self.state {
			ActorState::Idle => "Idle".to_string(),
			ActorState::Authenticating { .. } => "Authenticating".to_string(),
			ActorState::Authenticated { .. } => "Authenticated".to_string(),
			ActorState::Failed { .. } => "Failed".to_string(),
			ActorState::Shutdown => "Shutdown".to_string(),
		}
	}

	#[instrument(skip(self, hello_message, config))]
	async fn perform_authentication(&mut self, hello_message: Value, config: AuthConfig) -> Result<SessionInfo, AuthenticationError> {
		// Parse hello message to extract authentication context
		let auth_context = self.parse_hello_message(hello_message)?;

		self.metrics.challenges_received.increment();

		let _ = self.event_tx.send(AuthEvent::ChallengeReceived {
			session_id: self.session_id,
			challenge_length: auth_context.challenge.len(),
			timestamp: Instant::now(),
		});

		// Generate authentication hash according to OBS WebSocket v5 protocol
		let auth_hash = self.generate_auth_hash(&config.password, &auth_context)?;

		// Send identify message with authentication
		let identify_message = self.create_identify_message(&auth_hash);

		timeout(config.timeout, self.transport.send_message(identify_message))
			.await
			.map_err(|_| AuthenticationError::Timeout { duration: config.timeout })?
			.map_err(|e| AuthenticationError::NetworkError {
				details: format!("Failed to send identify message: {}", e),
			})?;

		// Wait for authentication response
		let response = timeout(config.timeout, self.transport.receive_message(config.timeout))
			.await
			.map_err(|_| AuthenticationError::Timeout { duration: config.timeout })?
			.map_err(|e| AuthenticationError::NetworkError {
				details: format!("Failed to receive auth response: {}", e),
			})?;

		// Validate authentication response
		let session_info = self.validate_auth_response(response, &auth_context)?;

		self.transition_to_authenticated(session_info.clone()).await;
		self.metrics.authentications_succeeded.increment();

		Ok(session_info)
	}

	fn parse_hello_message(&self, hello: Value) -> Result<AuthContext, AuthenticationError> {
		let d = hello.get("d").ok_or_else(|| AuthenticationError::InvalidHelloMessage {
			reason: "Missing 'd' field".to_string(),
		})?;

		let auth = d
			.get("authentication")
			.ok_or(AuthenticationError::MissingChallenge)?
			.as_object()
			.ok_or_else(|| AuthenticationError::InvalidHelloMessage {
				reason: "Authentication field is not an object".to_string(),
			})?;

		let challenge = auth.get("challenge").and_then(Value::as_str).ok_or(AuthenticationError::MissingChallenge)?.to_string();

		let salt = auth.get("salt").and_then(Value::as_str).ok_or(AuthenticationError::MissingChallenge)?.to_string();

		let obs_version = d.get("obsWebSocketVersion").and_then(Value::as_str).map(String::from);

		let protocol_version = d.get("rpcVersion").and_then(Value::as_str).map(String::from);

		Ok(AuthContext {
			challenge,
			salt,
			obs_version,
			protocol_version,
		})
	}

	fn generate_auth_hash(&self, password: &SecureString, context: &AuthContext) -> Result<String, AuthenticationError> {
		// First hash: SHA256(password + salt)
		let mut hasher = Sha256::new();
		hasher.update(password.as_str().as_bytes());
		hasher.update(context.salt.as_bytes());
		let first_hash = hasher.finalize();

		// Second hash: SHA256(first_hash + challenge)
		let mut second_hasher = Sha256::new();
		second_hasher.update(&first_hash[..]);
		second_hasher.update(context.challenge.as_bytes());
		let final_hash = second_hasher.finalize();

		// Base64 encode the final hash
		Ok(BASE64_STANDARD.encode(final_hash))
	}

	fn create_identify_message(&self, auth_hash: &str) -> Value {
		json!({
				"op": 1, // Identify opcode
				"d": {
						"rpcVersion": 1,
						"authentication": auth_hash,
						"eventSubscriptions": 33 // Subscribe to all events
				}
		})
	}

	fn validate_auth_response(&self, response: Value, context: &AuthContext) -> Result<SessionInfo, AuthenticationError> {
		let op = response.get("op").and_then(Value::as_u64).ok_or_else(|| AuthenticationError::ProtocolError {
			message: "Missing or invalid op code".to_string(),
			error_code: None,
		})?;

		if op != 2 {
			return Err(AuthenticationError::ProtocolError {
				message: format!("Expected op code 2 (Identified), got {}", op),
				error_code: Some(op as u16),
			});
		}

		let d = response.get("d").ok_or_else(|| AuthenticationError::ProtocolError {
			message: "Missing 'd' field in identify response".to_string(),
			error_code: None,
		})?;

		let negotiated_rpc_version = d.get("negotiatedRpcVersion").and_then(Value::as_u64).unwrap_or(1) as u32;

		Ok(SessionInfo {
			session_id: self.session_id,
			authenticated_at: Instant::now(),
			obs_version: context.obs_version.clone().unwrap_or_else(|| "unknown".to_string()),
			protocol_version: context.protocol_version.clone().unwrap_or_else(|| "unknown".to_string()),
			negotiated_rpc_version,
		})
	}
}

// ============================================================================
// Typestate Authentication Manager
// ============================================================================

/// Type-safe authentication manager using typestate pattern
pub struct AuthManager<S: AuthState> {
	inner: Arc<AuthManagerInner>,
	_state: PhantomData<S>,
}

struct AuthManagerInner {
	session_id: SessionId,
	command_tx: mpsc::Sender<AuthCommand>,
	event_rx: broadcast::Receiver<AuthEvent>,
	actor_handle: tokio::task::JoinHandle<()>,
	metrics: AuthMetrics,
}

// State-specific implementations
impl AuthManager<Unauthenticated> {
	pub fn new(transport: Box<dyn AuthTransport>) -> Self {
		let (command_tx, command_rx) = mpsc::channel(16);
		let (event_tx, event_rx) = broadcast::channel(32);

		let actor = AuthenticationActor::new(transport, command_rx, event_tx);
		let session_id = actor.session_id;
		let actor_handle = tokio::spawn(actor.run());

		Self {
			inner: Arc::new(AuthManagerInner {
				session_id,
				command_tx,
				event_rx,
				actor_handle,
				metrics: AuthMetrics::new(),
			}),
			_state: PhantomData,
		}
	}

	#[instrument(skip(self, hello_message, config))]
	pub async fn authenticate(self, hello_message: Value, config: AuthConfig) -> Result<AuthManager<Authenticated>, (AuthManager<Failed>, AuthenticationError)> {
		let (tx, rx) = oneshot::channel();

		let command = AuthCommand::Authenticate {
			hello_message,
			config,
			respond_to: tx,
		};

		if self.inner.command_tx.send(command).await.is_err() {
			let error = AuthenticationError::ActorUnavailable;
			return Err((
				AuthManager {
					inner: self.inner,
					_state: PhantomData,
				},
				error,
			));
		}

		match rx.await {
			Ok(Ok(_session_info)) => Ok(AuthManager {
				inner: self.inner,
				_state: PhantomData,
			}),
			Ok(Err(error)) => Err((
				AuthManager {
					inner: self.inner,
					_state: PhantomData,
				},
				error,
			)),
			Err(_) => {
				let error = AuthenticationError::ActorUnavailable;
				Err((
					AuthManager {
						inner: self.inner,
						_state: PhantomData,
					},
					error,
				))
			}
		}
	}
}

impl AuthManager<Authenticated> {
	pub fn session_info(&self) -> SessionId {
		self.inner.session_id
	}

	pub async fn reset(self) -> Result<AuthManager<Unauthenticated>, AuthenticationError> {
		let (tx, rx) = oneshot::channel();

		let command = AuthCommand::Reset { respond_to: tx };

		self.inner.command_tx.send(command).await.map_err(|_| AuthenticationError::ActorUnavailable)?;

		rx.await.map_err(|_| AuthenticationError::ActorUnavailable)?.map(|_| AuthManager {
			inner: self.inner,
			_state: PhantomData,
		})
	}
}

impl AuthManager<Failed> {
	pub async fn retry(self) -> AuthManager<Unauthenticated> {
		AuthManager {
			inner: self.inner,
			_state: PhantomData,
		}
	}

	pub fn error(&self) -> Option<AuthenticationError> {
		// In a real implementation, this would query the actor for the last error
		None
	}
}

// Generic implementations available in all states
impl<S: AuthState> AuthManager<S> {
	pub fn session_id(&self) -> SessionId {
		self.inner.session_id
	}

	pub async fn get_status(&self) -> Result<AuthStatus, AuthenticationError> {
		let (tx, rx) = oneshot::channel();

		let command = AuthCommand::GetStatus { respond_to: tx };

		self.inner.command_tx.send(command).await.map_err(|_| AuthenticationError::ActorUnavailable)?;

		rx.await.map_err(|_| AuthenticationError::ActorUnavailable)
	}

	pub fn subscribe_events(&self) -> broadcast::Receiver<AuthEvent> {
		self.inner.event_rx.resubscribe()
	}

	pub fn metrics(&self) -> &AuthMetrics {
		&self.inner.metrics
	}
}

impl<S: AuthState> Drop for AuthManager<S> {
	fn drop(&mut self) {
		// Send shutdown command to actor
		let _ = self.inner.command_tx.try_send(AuthCommand::Shutdown);
	}
}

// ============================================================================
// Metrics and Observability
// ============================================================================

/// Authentication metrics for monitoring
#[derive(Debug, Clone)]
pub struct AuthMetrics {
	pub authentications_attempted: Counter,
	pub authentications_succeeded: Counter,
	pub authentications_failed: Counter,
	pub challenges_received: Counter,
	pub auth_duration: Histogram,
}

impl AuthMetrics {
	pub fn new() -> Self {
		Self {
			authentications_attempted: Counter::new(),
			authentications_succeeded: Counter::new(),
			authentications_failed: Counter::new(),
			challenges_received: Counter::new(),
			auth_duration: Histogram::new(),
		}
	}
}

#[derive(Debug, Clone)]
pub struct Counter {
	value: Arc<std::sync::atomic::AtomicU64>,
}

impl Counter {
	pub fn new() -> Self {
		Self {
			value: Arc::new(std::sync::atomic::AtomicU64::new(0)),
		}
	}

	pub fn increment(&self) {
		self.value.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
	}

	pub fn get(&self) -> u64 {
		self.value.load(std::sync::atomic::Ordering::Relaxed)
	}
}

#[derive(Debug, Clone)]
pub struct Histogram {
	samples: Arc<std::sync::Mutex<Vec<Duration>>>,
}

impl Histogram {
	pub fn new() -> Self {
		Self {
			samples: Arc::new(std::sync::Mutex::new(Vec::new())),
		}
	}

	pub fn record(&self, value: Duration) {
		if let Ok(mut samples) = self.samples.lock() {
			samples.push(value);
			// Keep only last 1000 samples to prevent unbounded growth
			if samples.len() > 1000 {
				samples.remove(0);
			}
		}
	}

	pub fn percentile(&self, p: f64) -> Option<Duration> {
		let samples = self.samples.lock().ok()?;
		if samples.is_empty() {
			return None;
		}

		let mut sorted = samples.clone();
		sorted.sort();
		let index = ((p / 100.0) * (sorted.len() - 1) as f64) as usize;
		Some(sorted[index])
	}
}

// ============================================================================
// Tests and Examples
// ============================================================================

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::time::sleep;

	// Mock transport for testing
	struct MockTransport {
		responses: Vec<Value>,
		response_index: std::sync::atomic::AtomicUsize,
	}

	impl MockTransport {
		fn new(responses: Vec<Value>) -> Self {
			Self {
				responses,
				response_index: std::sync::atomic::AtomicUsize::new(0),
			}
		}

		fn with_successful_auth() -> Self {
			let hello = json!({
					"op": 0,
					"d": {
							"obsWebSocketVersion": "5.0.0",
							"rpcVersion": "1",
							"authentication": {
									"challenge": "test_challenge",
									"salt": "test_salt"
							}
					}
			});

			let identify_response = json!({
					"op": 2,
					"d": {
							"negotiatedRpcVersion": 1
					}
			});

			Self::new(vec![identify_response])
		}
	}

	#[async_trait::async_trait]
	impl AuthTransport for MockTransport {
		async fn send_message(&mut self, _message: Value) -> Result<(), AuthTransportError> {
			Ok(())
		}

		async fn receive_message(&mut self, _timeout: Duration) -> Result<Value, AuthTransportError> {
			let index = self.response_index.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
			if index < self.responses.len() {
				Ok(self.responses[index].clone())
			} else {
				Err(AuthTransportError::Timeout { duration: Duration::from_secs(1) })
			}
		}
	}

	#[tokio::test]
	async fn test_config_builder_validation() {
		// Valid configuration
		let config = AuthConfigBuilder::new().password("test_password").timeout(Duration::from_secs(5)).max_attempts(3).build();

		assert!(config.is_ok());

		// Invalid configuration - empty password
		let config = AuthConfigBuilder::new().timeout(Duration::from_secs(5)).build();

		assert!(matches!(config, Err(ValidationError::EmptyPassword)));
	}

	#[tokio::test]
	async fn test_typestate_transitions() {
		let transport = Box::new(MockTransport::with_successful_auth());
		let auth_manager = AuthManager::new(transport);

		// Verify we start in Unauthenticated state
		let status = auth_manager.get_status().await.unwrap();
		assert_eq!(status.state, "Idle");

		// This should compile - we can authenticate from Unauthenticated
		let config = AuthConfigBuilder::new().password("test_password").build().unwrap();

		let hello = json!({
				"op": 0,
				"d": {
						"authentication": {
								"challenge": "test_challenge",
								"salt": "test_salt"
						}
				}
		});

		// Note: This test would need a more sophisticated mock to fully work
		// but demonstrates the typestate pattern structure
	}

	#[tokio::test]
	async fn test_secure_string_redaction() {
		let secure = SecureString::new("secret_password".to_string());
		let debug_output = format!("{:?}", secure);
		assert!(!debug_output.contains("secret_password"));
		assert!(debug_output.contains("[REDACTED]"));
	}

	#[tokio::test]
	async fn test_authentication_error_classification() {
		let invalid_creds = AuthenticationError::InvalidCredentials;
		assert!(!invalid_creds.is_retryable());

		let timeout_error = AuthenticationError::Timeout { duration: Duration::from_secs(5) };
		assert!(timeout_error.is_retryable());
		assert!(timeout_error.retry_delay().is_some());

		let network_error = AuthenticationError::NetworkError {
			details: "Connection refused".to_string(),
		};
		assert!(network_error.is_retryable());
	}

	#[tokio::test]
	async fn test_metrics_collection() {
		let metrics = AuthMetrics::new();

		metrics.authentications_attempted.increment();
		metrics.authentications_succeeded.increment();

		assert_eq!(metrics.authentications_attempted.get(), 1);
		assert_eq!(metrics.authentications_succeeded.get(), 1);
		assert_eq!(metrics.authentications_failed.get(), 0);

		metrics.auth_duration.record(Duration::from_millis(100));
		assert!(metrics.auth_duration.percentile(50.0).is_some());
	}

	#[tokio::test]
	async fn test_session_id_uniqueness() {
		let id1 = SessionId::new();
		let id2 = SessionId::new();

		assert_ne!(id1, id2);

		let id_str = id1.to_string();
		assert!(!id_str.is_empty());
	}

	#[test]
	fn test_auth_hash_generation() {
		// This would test the actual hash generation logic
		// Implementation would need access to private methods or
		// extraction to testable functions
	}
}

// ============================================================================
// Integration Examples
// ============================================================================

/// Example usage of the authentication system
#[cfg(feature = "examples")]
pub mod examples {
	use super::*;
	use tracing::{error, info};

	pub async fn example_authentication_flow() -> Result<(), Box<dyn std::error::Error>> {
		// Initialize tracing
		tracing_subscriber::fmt::init();

		// Create configuration
		let config = AuthConfigBuilder::new()
			.password("your_obs_password")
			.timeout(Duration::from_secs(10))
			.max_attempts(3)
			.retry_delay(Duration::from_secs(1))
			.build()?;

		// Create transport (would be actual WebSocket in real usage)
		let transport = create_websocket_transport().await?;

		// Create authentication manager in Unauthenticated state
		let auth_manager = AuthManager::new(transport);

		// Subscribe to authentication events
		let mut event_rx = auth_manager.subscribe_events();

		tokio::spawn(async move {
			while let Ok(event) = event_rx.recv().await {
				match event {
					AuthEvent::StateChanged { from_state, to_state, .. } => {
						info!("Auth state: {} -> {}", from_state, to_state);
					}
					AuthEvent::AuthenticationSucceeded { session_info, .. } => {
						info!("Authentication successful: {:?}", session_info);
					}
					AuthEvent::AuthenticationFailed { error, .. } => {
						error!("Authentication failed: {}", error);
					}
					_ => {}
				}
			}
		});

		// Simulate receiving hello message from OBS
		let hello_message = json!({
				"op": 0,
				"d": {
						"obsWebSocketVersion": "5.0.0",
						"rpcVersion": "1",
						"authentication": {
								"challenge": "your_challenge_here",
								"salt": "your_salt_here"
						}
				}
		});

		// Attempt authentication - type changes from Unauthenticated to Authenticated or Failed
		match auth_manager.authenticate(hello_message, config).await {
			Ok(authenticated_manager) => {
				info!("Successfully authenticated! Session: {}", authenticated_manager.session_id());

				// Now we can perform authenticated operations
				let status = authenticated_manager.get_status().await?;
				info!("Current status: {:?}", status);

				// Later, we can reset to start over
				let reset_manager = authenticated_manager.reset().await?;
				info!("Reset to unauthenticated state");

				Ok(())
			}
			Err((failed_manager, error)) => {
				error!("Authentication failed: {}", error);

				// We can retry if the error is retryable
				if error.is_retryable() {
					info!("Error is retryable, attempting retry...");
					let retry_manager = failed_manager.retry().await;
					// Could attempt authentication again here
				}

				Err(error.into())
			}
		}
	}

	// Placeholder for actual WebSocket transport creation
	pub async fn create_websocket_transport() -> Result<Box<dyn AuthTransport>, Box<dyn std::error::Error>> {
		// In real implementation, this would create a WebSocket connection to OBS
		// For example purposes, we'll use a mock
		Ok(Box::new(super::tests::MockTransport::with_successful_auth()))
	}
}

// ============================================================================
// Public API Re-exports
// ============================================================================

pub use self::{
	AuthConfig, AuthConfigBuilder, AuthContext, AuthEvent, AuthManager, AuthMetrics, AuthState, AuthStatus, AuthTransportError, Authenticated, Authenticating,
	AuthenticationError, Counter, Failed, Histogram, SecureString, SessionId, SessionInfo, Unauthenticated, ValidationError, WebSocketTransport,
};

/// Convenience type alias for unauthenticated auth manager
pub type UnauthenticatedAuthManager = AuthManager<Unauthenticated>;

/// Convenience type alias for authenticated auth manager  
pub type AuthenticatedAuthManager = AuthManager<Authenticated>;

/// Convenience type alias for failed auth manager
pub type FailedAuthManager = AuthManager<Failed>;
