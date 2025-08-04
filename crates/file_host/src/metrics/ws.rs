use lazy_static::lazy_static;
use prometheus::{register_gauge_vec, register_histogram_vec, register_int_counter_vec, register_int_gauge_vec, GaugeVec, HistogramVec, IntCounterVec, IntGaugeVec};

lazy_static! {
		// Connection lifecycle metrics
		pub static ref WS_CONNECTIONS_TOTAL: IntCounterVec = register_int_counter_vec!(
				"ws_connections_total",
				"Total WebSocket connections created",
				&["outcome"] // "created", "removed"
		).expect("Failed to register WS_CONNECTIONS_TOTAL");

		pub static ref WS_CONNECTION_DURATION: HistogramVec = register_histogram_vec!(
				"ws_connection_duration_seconds",
				"Duration of WebSocket connections",
				&["reason"] // "timeout", "client_close", "error", "normal"
		).expect("Failed to register WS_CONNECTION_DURATION");

		pub static ref WS_CONNECTIONS_ACTIVE: IntGaugeVec = register_int_gauge_vec!(
				"ws_connections_active",
				"Currently active WebSocket connections",
				&["state"] // "active", "stale", "disconnected"
		).expect("Failed to register WS_CONNECTIONS_ACTIVE");

		// Message processing metrics
		pub static ref WS_MESSAGES_TOTAL: IntCounterVec = register_int_counter_vec!(
				"ws_messages_total",
				"Total WebSocket messages processed",
				&["type", "result"] // type: "ping", "pong", "subscribe", "broadcast", etc. result: "success", "failed"
		).expect("Failed to register WS_MESSAGES_TOTAL");

		pub static ref WS_MESSAGE_PROCESSING_DURATION: HistogramVec = register_histogram_vec!(
				"ws_message_processing_duration_seconds",
				"Time taken to process WebSocket messages",
				&["type", "stage"] // stage: "parse", "validate", "process", "broadcast"
		).expect("Failed to register WS_MESSAGE_PROCESSING_DURATION");

		// Broadcast metrics
		pub static ref WS_BROADCAST_OPERATIONS: IntCounterVec = register_int_counter_vec!(
				"ws_broadcast_operations_total",
				"Total broadcast operations",
				&["event_type", "result"] // result: "success", "failed", "no_subscribers"
		).expect("Failed to register WS_BROADCAST_OPERATIONS");

		pub static ref WS_BROADCAST_DELIVERY: IntCounterVec = register_int_counter_vec!(
				"ws_broadcast_delivery_total",
				"Total messages delivered/failed in broadcasts",
				&["event_type", "outcome"] // outcome: "delivered", "failed"
		).expect("Failed to register WS_BROADCAST_DELIVERY");

		pub static ref WS_BROADCAST_DURATION: HistogramVec = register_histogram_vec!(
				"ws_broadcast_duration_seconds",
				"Time taken for broadcast operations",
				&["event_type"]
		).expect("Failed to register WS_BROADCAST_DURATION");

		// Subscription metrics
		pub static ref WS_SUBSCRIPTIONS: IntGaugeVec = register_int_gauge_vec!(
				"ws_subscriptions_active",
				"Active subscriptions by event type",
				&["event_type"]
		).expect("Failed to register WS_SUBSCRIPTIONS");

		pub static ref WS_SUBSCRIPTION_OPERATIONS: IntCounterVec = register_int_counter_vec!(
				"ws_subscription_operations_total",
				"Subscription operations",
				&["operation", "event_type"] // operation: "subscribe", "unsubscribe"
		).expect("Failed to register WS_SUBSCRIPTION_OPERATIONS");

		// Health and invariant metrics
		pub static ref WS_INVARIANT_VIOLATIONS: IntCounterVec = register_int_counter_vec!(
				"ws_invariant_violations_total",
				"Invariant violations detected",
				&["invariant_type"] // "connection_count", "state_consistency", "resource_leak"
		).expect("Failed to register WS_INVARIANT_VIOLATIONS");

		pub static ref WS_HEALTH_CHECKS: IntCounterVec = register_int_counter_vec!(
				"ws_health_checks_total",
				"Health check operations",
				&["check_type", "result"] // check_type: "timeout_monitor", "connection_cleanup", "metrics_validation"
		).expect("Failed to register WS_HEALTH_CHECKS");

		pub static ref WS_RESOURCE_USAGE: GaugeVec = register_gauge_vec!(
				"ws_resource_usage",
				"Resource usage metrics",
				&["resource_type"] // "memory_connections", "channel_capacity", "pending_messages"
		).expect("Failed to register WS_RESOURCE_USAGE");

		// Error metrics
		pub static ref WS_ERRORS_TOTAL: IntCounterVec = register_int_counter_vec!(
				"ws_errors_total",
				"WebSocket errors by type",
				&["error_type", "component"] // component: "connection", "message", "broadcast", "subscription"
		).expect("Failed to register WS_ERRORS_TOTAL");

		// System event metrics
		pub static ref WS_SYSTEM_EVENTS: IntCounterVec = register_int_counter_vec!(
				"ws_system_events_total",
				"System events emitted",
				&["event_type"] // "state_change", "message_processed", "broadcast_failed", "cleanup"
		).expect("Failed to register WS_SYSTEM_EVENTS");
}

/// Macro for timing WebSocket operations with automatic metrics recording
#[macro_export]
macro_rules! timed_ws_operation {
	($operation_type:expr, $stage:expr, $body:block) => {{
		let start = std::time::Instant::now();
		let result = $body;
		let duration = start.elapsed().as_secs_f64();

		$crate::metrics::WS_MESSAGE_PROCESSING_DURATION
			.with_label_values(&[$operation_type, $stage])
			.observe(duration);

		tracing::debug!(
			operation_type = $operation_type,
			stage = $stage,
			duration_ms = duration * 1000.0,
			"WebSocket operation completed"
		);

		result
	}};
}

/// Macro for recording connection lifecycle events
#[macro_export]
macro_rules! record_connection_event {
    ($event_type:expr, $connection_id:expr) => {
        $crate::metrics::WS_CONNECTIONS_TOTAL
            .with_label_values(&[$event_type])
            .inc();

        tracing::info!(
            event_type = $event_type,
            connection_id = %$connection_id,
            "Connection lifecycle event"
        );
    };

    ($event_type:expr, $connection_id:expr, duration: $duration:expr, reason: $reason:expr) => {
        $crate::metrics::WS_CONNECTIONS_TOTAL
            .with_label_values(&[$event_type])
            .inc();

        $crate::metrics::WS_CONNECTION_DURATION
            .with_label_values(&[$reason])
            .observe($duration.as_secs_f64());

        tracing::info!(
            event_type = $event_type,
            connection_id = %$connection_id,
            duration_ms = $duration.as_millis(),
            reason = $reason,
            "Connection lifecycle event with duration"
        );
    };
}

/// Macro for updating connection state metrics
#[macro_export]
macro_rules! update_connection_state {
	($from_state:expr, $to_state:expr) => {
		// Decrement old state
		match $from_state {
			"active" => $crate::metrics::WS_CONNECTIONS_ACTIVE.with_label_values(&["active"]).dec(),
			"stale" => $crate::metrics::WS_CONNECTIONS_ACTIVE.with_label_values(&["stale"]).dec(),
			"disconnected" => $crate::metrics::WS_CONNECTIONS_ACTIVE.with_label_values(&["disconnected"]).dec(),
			_ => {}
		}

		// Increment new state
		match $to_state {
			"active" => $crate::metrics::WS_CONNECTIONS_ACTIVE.with_label_values(&["active"]).inc(),
			"stale" => $crate::metrics::WS_CONNECTIONS_ACTIVE.with_label_values(&["stale"]).inc(),
			"disconnected" => $crate::metrics::WS_CONNECTIONS_ACTIVE.with_label_values(&["disconnected"]).inc(),
			_ => {}
		}

		tracing::debug!(from_state = $from_state, to_state = $to_state, "Connection state updated");
	};
}

/// Macro for recording message processing results
#[macro_export]
macro_rules! record_message_result {
    ($message_type:expr, $result:expr) => {
        $crate::metrics::WS_MESSAGES_TOTAL
            .with_label_values(&[$message_type, $result])
            .inc();
    };

    ($message_type:expr, $result:expr, connection_id: $connection_id:expr) => {
        $crate::metrics::WS_MESSAGES_TOTAL
            .with_label_values(&[$message_type, $result])
            .inc();

        tracing::debug!(
            message_type = $message_type,
            result = $result,
            connection_id = %$connection_id,
            "Message processing result recorded"
        );
    };
}

/// Macro for timing and recording broadcast operations
#[macro_export]
macro_rules! timed_broadcast {
	($event_type:expr, $body:block) => {{
		let start = std::time::Instant::now();
		let result = $body;
		let duration = start.elapsed().as_secs_f64();

		$crate::metrics::WS_BROADCAST_DURATION.with_label_values(&[$event_type]).observe(duration);

		// Record the broadcast operation result
		match &result {
			Ok(process_result) => {
				$crate::metrics::WS_BROADCAST_OPERATIONS
					.with_label_values(&[$event_type, if process_result.delivered > 0 { "success" } else { "no_subscribers" }])
					.inc();

				$crate::metrics::WS_BROADCAST_DELIVERY
					.with_label_values(&[$event_type, "delivered"])
					.inc_by(process_result.delivered as u64);

				if process_result.failed > 0 {
					$crate::metrics::WS_BROADCAST_DELIVERY
						.with_label_values(&[$event_type, "failed"])
						.inc_by(process_result.failed as u64);
				}
			}
			Err(_) => {
				$crate::metrics::WS_BROADCAST_OPERATIONS.with_label_values(&[$event_type, "failed"]).inc();
			}
		}

		tracing::debug!(
			event_type = $event_type,
			duration_ms = duration * 1000.0,
			success = result.is_ok(),
			"Broadcast operation completed"
		);

		result
	}};
}

/// Macro for recording subscription changes
#[macro_export]
macro_rules! record_subscription_change {
    ($operation:expr, $event_types:expr, $changed_count:expr, $connection_id:expr) => {
        for event_type in $event_types {
            $crate::metrics::WS_SUBSCRIPTION_OPERATIONS
                .with_label_values(&[$operation, &format!("{:?}", event_type)])
                .inc();

            // Update active subscription gauge
            let event_type_str = format!("{:?}", event_type);
            match $operation {
                "subscribe" => $crate::metrics::WS_SUBSCRIPTIONS
                    .with_label_values(&[&event_type_str])
                    .inc(),
                "unsubscribe" => $crate::metrics::WS_SUBSCRIPTIONS
                    .with_label_values(&[&event_type_str])
                    .dec(),
                _ => {}
            }
        }

        tracing::debug!(
            operation = $operation,
            changed_count = $changed_count,
            connection_id = %$connection_id,
            event_types = ?$event_types,
            "Subscription change recorded"
        );
    };
}

/// Macro for checking and recording invariant violations
#[macro_export]
macro_rules! check_invariant {
	($condition:expr, $invariant_type:expr, $description:expr) => {
		if !$condition {
			$crate::metrics::WS_INVARIANT_VIOLATIONS.with_label_values(&[$invariant_type]).inc();

			tracing::error!(invariant_type = $invariant_type, description = $description, "Invariant violation detected");
		}
	};

	($condition:expr, $invariant_type:expr, $description:expr, expected: $expected:expr, actual: $actual:expr) => {
		if !$condition {
			$crate::metrics::WS_INVARIANT_VIOLATIONS.with_label_values(&[$invariant_type]).inc();

			tracing::error!(
				invariant_type = $invariant_type,
				description = $description,
				expected = $expected,
				actual = $actual,
				"Invariant violation detected with values"
			);
		}
	};
}

/// Macro for health check operations
#[macro_export]
macro_rules! health_check {
	($check_type:expr, $body:block) => {{
		let start = std::time::Instant::now();
		let result = $body;
		let duration = start.elapsed();

		let result_label = if result.is_ok() { "success" } else { "failed" };
		$crate::metrics::WS_HEALTH_CHECKS.with_label_values(&[$check_type, result_label]).inc();

		tracing::debug!(
			check_type = $check_type,
			result = result_label,
			duration_ms = duration.as_millis(),
			"Health check completed"
		);

		result
	}};
}

/// Macro for recording resource usage
#[macro_export]
macro_rules! update_resource_usage {
	($resource_type:expr, $value:expr) => {
		$crate::metrics::WS_RESOURCE_USAGE.with_label_values(&[$resource_type]).set($value);
	};
}

/// Macro for recording errors with context
#[macro_export]
macro_rules! record_ws_error {
    ($error_type:expr, $component:expr) => {
        $crate::metrics::WS_ERRORS_TOTAL
            .with_label_values(&[$error_type, $component])
            .inc();

        tracing::error!(
            error_type = $error_type,
            component = $component,
            "WebSocket error recorded"
        );
    };

    ($error_type:expr, $component:expr, $error:expr) => {
        $crate::metrics::WS_ERRORS_TOTAL
            .with_label_values(&[$error_type, $component])
            .inc();

        tracing::error!(
            error_type = $error_type,
            component = $component,
            error = %$error,
            "WebSocket error recorded with details"
        );
    };
}

/// Macro for recording system events
#[macro_export]
macro_rules! record_system_event {
    ($event_type:expr) => {
        $crate::metrics::WS_SYSTEM_EVENTS
            .with_label_values(&[$event_type])
            .inc();
    };

    ($event_type:expr, $($key:ident = $value:expr),+) => {
        $crate::metrics::WS_SYSTEM_EVENTS
            .with_label_values(&[$event_type])
            .inc();

        tracing::info!(
            event_type = $event_type,
            $($key = $value),+,
            "System event recorded"
        );
    };
}

/// Macro for comprehensive connection cleanup instrumentation
#[macro_export]
macro_rules! cleanup_connection {
    ($connection_id:expr, $reason:expr, $duration:expr, $resources_freed:expr) => {
        record_connection_event!("removed", $connection_id, duration: $duration, reason: $reason);
        record_system_event!("cleanup",
            connection_id = %$connection_id,
            reason = $reason,
            resources_freed = $resources_freed
        );

        update_resource_usage!("memory_connections", -1.0);
    };
}

/// Macro for periodic health metrics snapshot
#[macro_export]
macro_rules! log_health_snapshot {
    ($metrics:expr, $connection_count:expr) => {
        let snapshot = $metrics.get_snapshot();

        update_resource_usage!("active_connections", snapshot.current_active as f64);
        update_resource_usage!("stale_connections", snapshot.current_stale as f64);
        update_resource_usage!("total_connections", $connection_count as f64);

        tracing::info!(
            total_connections = $connection_count,
            active_connections = snapshot.current_active,
            stale_connections = snapshot.current_stale,
            messages_processed = snapshot.messages_processed,
            messages_failed = snapshot.messages_failed,
            broadcast_succeeded = snapshot.broadcast_succeeded,
            broadcast_failed = snapshot.broadcast_failed,
            "Health snapshot logged"
        );

        // Check key invariants
        check_invariant!(
            $connection_count as u64 == (snapshot.total_created - snapshot.total_removed),
            "connection_count",
            "Active connection count mismatch",
            expected: (snapshot.total_created - snapshot.total_removed),
            actual: $connection_count as u64
        );
    };
}
