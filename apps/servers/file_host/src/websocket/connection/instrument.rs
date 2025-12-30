use lazy_static::lazy_static;
use prometheus::{register_histogram_vec, register_int_counter_vec, register_int_gauge_vec, HistogramVec, IntCounterVec, IntGaugeVec};

lazy_static! {
		// Connection-specific metrics
		pub static ref CONNECTION_LIFECYCLE: IntCounterVec = register_int_counter_vec!(
				"ws_connection_lifecycle_total",
				"Connection lifecycle events",
				&["event"] // "created", "marked_stale", "disconnected", "removed"
		).expect("Failed to register CONNECTION_LIFECYCLE");

		pub static ref CONNECTION_STATES: IntGaugeVec = register_int_gauge_vec!(
				"ws_connection_states",
				"Current connection counts by state",
				&["state"] // "active", "stale"
		).expect("Failed to register CONNECTION_STATES");

		pub static ref CONNECTION_DURATION: HistogramVec = register_histogram_vec!(
				"ws_connection_duration_seconds",
				"Connection lifetime duration",
				&["end_reason"], // "timeout", "client_disconnect", "error", "cleanup"
				vec![1.0, 5.0, 30.0, 60.0, 300.0, 600.0, 1800.0, 3600.0]
		).expect("Failed to register CONNECTION_DURATION");

		pub static ref CLIENT_CONNECTIONS: IntGaugeVec = register_int_gauge_vec!(
				"ws_client_connections",
				"Active connections per client",
				&["client_type"] // "auth", "proxy", "direct"
		).expect("Failed to register CLIENT_CONNECTIONS");

		pub static ref CONNECTION_MESSAGES: IntCounterVec = register_int_counter_vec!(
				"ws_connection_messages_total",
				"Messages processed per connection",
				&["message_type"] // "ping", "pong", "subscribe", "unsubscribe", "broadcast"
		).expect("Failed to register CONNECTION_MESSAGES");

		pub static ref TIMEOUT_MONITOR_OPERATIONS: IntCounterVec = register_int_counter_vec!(
				"ws_timeout_monitor_operations_total",
				"Timeout monitor operations",
				&["operation", "result"] // operation: "mark_stale", "cleanup", "health_check"
		).expect("Failed to register TIMEOUT_MONITOR_OPERATIONS");

		pub static ref CONNECTION_SUBSCRIPTIONS: IntGaugeVec = register_int_gauge_vec!(
				"ws_connection_subscriptions",
				"Subscription counts by event type",
				&["event_type"]
		).expect("Failed to register CONNECTION_SUBSCRIPTIONS");

		pub static ref CONNECTION_ERRORS: IntCounterVec = register_int_counter_vec!(
				"ws_connection_errors_total",
				"Connection-related errors",
				&["error_type", "phase"] // phase: "creation", "operation", "cleanup"
		).expect("Failed to register CONNECTION_ERRORS");
}

/// Records connection creation
#[macro_export]
macro_rules! record_connection_created {
    ($connection_id:expr, $client_id:expr) => {
        $crate::websocket::connection::instrument::CONNECTION_LIFECYCLE
            .with_label_values(&["created"])
            .inc();

        $crate::websocket::connection::instrument::CONNECTION_STATES
            .with_label_values(&["active"])
            .inc();

        // Track client type distribution
        let client_type = if $client_id.as_str().starts_with("auth:") {
            "auth"
        } else if $client_id.as_str().starts_with("proxy:") {
            "proxy"
        } else {
            "direct"
        };

        $crate::websocket::connection::instrument::CLIENT_CONNECTIONS
            .with_label_values(&[client_type])
            .inc();

        info!(
            connection_id = %$connection_id,
            client_id = %$client_id,
            client_type = client_type,
            "Connection created"
        );
    };
}

/// Records connection state transitions
#[macro_export]
macro_rules! record_connection_state_change {
    ($connection_id:expr, $client_id:expr, $from_state:expr, $to_state:expr) => {
        match (&$from_state, &$to_state) {
            (ConnectionState::Active { .. }, ConnectionState::Stale { .. }) => {
                $crate::websocket::connection::instrument::CONNECTION_LIFECYCLE
                    .with_label_values(&["marked_stale"])
                    .inc();

                $crate::websocket::connection::instrument::CONNECTION_STATES
                    .with_label_values(&["active"])
                    .dec();

                $crate::websocket::connection::instrument::CONNECTION_STATES
                    .with_label_values(&["stale"])
                    .inc();

                info!(
                    connection_id = %$connection_id,
                    client_id = %$client_id,
                    "Connection marked as stale"
                );
            },
            (ConnectionState::Active { .. }, ConnectionState::Disconnected { .. }) => {
                $crate::websocket::connection::instrument::CONNECTION_LIFECYCLE
                    .with_label_values(&["disconnected"])
                    .inc();

                $crate::websocket::connection::instrument::CONNECTION_STATES
                    .with_label_values(&["active"])
                    .dec();

                info!(
                    connection_id = %$connection_id,
                    client_id = %$client_id,
                    "Connection disconnected from active state"
                );
            },
            (ConnectionState::Stale { .. }, ConnectionState::Disconnected { .. }) => {
                $crate::websocket::connection::instrument::CONNECTION_LIFECYCLE
                    .with_label_values(&["disconnected"])
                    .inc();

                $crate::websocket::connection::instrument::CONNECTION_STATES
                    .with_label_values(&["stale"])
                    .dec();

                info!(
                    connection_id = %$connection_id,
                    client_id = %$client_id,
                    "Connection disconnected from stale state"
                );
            },
            _ => {
                warn!(
                    connection_id = %$connection_id,
                    client_id = %$client_id,
                    from_state = ?$from_state,
                    to_state = ?$to_state,
                    "Unexpected state transition"
                );
            }
        }
    };
}

/// Records connection removal with duration tracking
#[macro_export]
macro_rules! record_connection_removed {
    ($connection_id:expr, $client_id:expr, $duration:expr, $reason:expr) => {
        $crate::websocket::connection::instrument::CONNECTION_LIFECYCLE
            .with_label_values(&["removed"])
            .inc();

        // Determine end reason category for histogram
        let reason_category = if $reason.contains("timeout") || $reason.contains("stale") {
            "timeout"
        } else if $reason.contains("closed") || $reason.contains("disconnect") {
            "client_disconnect"
        } else if $reason.contains("error") || $reason.contains("failed") {
            "error"
        } else {
            "cleanup"
        };

        $crate::websocket::connection::instrument::CONNECTION_DURATION
            .with_label_values(&[reason_category])
            .observe($duration.as_secs_f64());

        // Update client connection count
        let client_type = if $client_id.as_str().starts_with("auth:") {
            "auth"
        } else if $client_id.as_str().starts_with("proxy:") {
            "proxy"
        } else {
            "direct"
        };

        $crate::websocket::connection::instrument::CLIENT_CONNECTIONS
            .with_label_values(&[client_type])
            .dec();

        info!(
            connection_id = %$connection_id,
            client_id = %$client_id,
            duration_secs = $duration.as_secs(),
            reason = $reason,
            reason_category = reason_category,
            "Connection removed"
        );
    };
}

/// Records message processing for a connection
#[macro_export]
macro_rules! record_connection_message {
    ($connection_id:expr, $message_type:expr) => {
        $crate::websocket::connection::instrument::CONNECTION_MESSAGES
            .with_label_values(&[$message_type])
            .inc();

        debug!(
            connection_id = %$connection_id,
            message_type = $message_type,
            "Message processed"
        );
    };
}

/// Records subscription changes
#[macro_export]
macro_rules! record_subscription_change {
    ($connection_id:expr, $operation:expr, $event_types:expr, $changed_count:expr) => {
        for event_type in $event_types {
            let event_type_str = format!("{:?}", event_type);
            match $operation {
                "subscribe" => {
                    $crate::websocket::connection::instrument::CONNECTION_SUBSCRIPTIONS
                        .with_label_values(&[&event_type_str])
                        .inc();
                },
                "unsubscribe" => {
                    $crate::websocket::connection::instrument::CONNECTION_SUBSCRIPTIONS
                        .with_label_values(&[&event_type_str])
                        .dec();
                },
                _ => {}
            }
        }

        debug!(
            connection_id = %$connection_id,
            operation = $operation,
            changed_count = $changed_count,
            event_types = ?$event_types,
            "Subscription change recorded"
        );
    };
}

/// Records timeout monitor operations
#[macro_export]
macro_rules! record_timeout_operation {
	($operation:expr, $result:expr, $count:expr) => {
		$crate::websocket::connection::instrument::TIMEOUT_MONITOR_OPERATIONS
			.with_label_values(&[$operation, $result])
			.inc();

		if $count > 0 {
			info!(operation = $operation, result = $result, count = $count, "Timeout monitor operation completed");
		}
	};
}

/// Records connection errors with context
#[macro_export]
macro_rules! record_connection_error {
    ($error_type:expr, $phase:expr, $error:expr) => {
        $crate::websocket::connection::instrument::CONNECTION_ERRORS
            .with_label_values(&[$error_type, $phase])
            .inc();

        error!(
            error_type = $error_type,
            phase = $phase,
            error = %$error,
            "Connection error recorded"
        );
    };

    ($error_type:expr, $phase:expr, $connection_id:expr, $error:expr) => {
        $crate::websocket::connection::instrument::CONNECTION_ERRORS
            .with_label_values(&[$error_type, $phase])
            .inc();

        error!(
            error_type = $error_type,
            phase = $phase,
            connection_id = %$connection_id,
            error = %$error,
            "Connection error recorded with context"
        );
    };
}

/// Health check for connection module invariants
#[macro_export]
macro_rules! connection_health_check {
	($connections:expr) => {{
		let active_count = $connections.iter().filter(|entry| entry.value().is_active()).count();

		let stale_count = $connections.iter().filter(|entry| entry.value().is_stale()).count();

		let total_count = $connections.len();

		// Update current state metrics to ensure accuracy
		$crate::connection::instrument::CONNECTION_STATES.with_label_values(&["active"]).set(active_count as i64);

		$crate::connection::instrument::CONNECTION_STATES.with_label_values(&["stale"]).set(stale_count as i64);

		// Log health snapshot
		debug!(
			total_connections = total_count,
			active_connections = active_count,
			stale_connections = stale_count,
			"Connection health check completed"
		);

		// Return health data for caller
		(total_count, active_count, stale_count)
	}};
}
