#[cfg(test)]
mod tests {
	use std::thread::sleep;
	use std::{
		net::SocketAddr,
		time::{Duration, Instant},
	};
	use ws_connection::core::conn::Connection;
	use ws_connection::types::*;

	// Helper function to create test connection
	fn create_test_connection() -> Connection {
		let client_id = ClientId::new("test-client");
		let addr = "127.0.0.1:8080".parse().unwrap();
		Connection::new(client_id, addr)
	}

	// Helper to create connection with custom client_id
	fn create_connection_with_client(client_id: &str) -> Connection {
		let addr = "127.0.0.1:8080".parse().unwrap();
		Connection::new(ClientId::new(client_id), addr)
	}

	#[test]
	fn test_new_connection_creation() {
		let client_id = ClientId::new("test-client");
		let addr = "127.0.0.1:8080".parse().unwrap();
		let conn = Connection::new(client_id.clone(), addr);

		assert_eq!(conn.client_id, client_id);
		assert_eq!(conn.source_addr, addr);
		assert!(conn.is_active());
		assert_eq!(conn.get_subscription_count(), 0);
	}

	#[test]
	fn test_connection_has_unique_ids() {
		let conn1 = create_test_connection();
		let conn2 = create_test_connection();

		assert_ne!(conn1.id, conn2.id);
	}

	#[test]
	fn test_new_connection_is_active() {
		let conn = create_test_connection();

		assert!(conn.is_active());
		assert!(!conn.is_stale());
		assert!(!conn.is_disconnected());

		match conn.state {
			ConnectionState::Active { .. } => (),
			_ => panic!("Expected Active state"),
		}
	}

	#[test]
	fn test_record_activity_updates_timestamp() {
		let mut conn = create_test_connection();

		sleep(Duration::from_millis(10));
		let before_activity = conn.last_activity;

		conn.record_activity();

		assert!(conn.last_activity > before_activity);
	}

	#[test]
	fn test_record_activity_updates_last_ping_when_active() {
		let mut conn = create_test_connection();

		sleep(Duration::from_millis(10));

		let initial_ping = match conn.state {
			ConnectionState::Active { last_ping } => last_ping,
			_ => panic!("Expected Active state"),
		};

		conn.record_activity();

		let new_ping = match conn.state {
			ConnectionState::Active { last_ping } => last_ping,
			_ => panic!("Expected Active state"),
		};

		assert!(new_ping > initial_ping);
	}

	#[test]
	fn test_record_activity_does_not_change_state_from_stale() {
		let mut conn = create_test_connection();
		conn.mark_stale("timeout".to_string());

		conn.record_activity();

		assert!(conn.is_stale());
	}

	#[test]
	fn test_subscribe_to_single_event() {
		let mut conn = create_test_connection();

		let change = conn.subscribe(vec!["user.created".to_string()]);

		assert_eq!(change.added, 1);
		assert_eq!(change.removed, 0);
		assert_eq!(change.total, 1);
		assert!(conn.is_subscribed_to(&"user.created".to_string()));
	}

	#[test]
	fn test_subscribe_to_multiple_events() {
		let mut conn = create_test_connection();

		let events = vec!["user.created".to_string(), "user.updated".to_string(), "user.deleted".to_string()];

		let change = conn.subscribe(events.clone());

		assert_eq!(change.added, 3);
		assert_eq!(change.total, 3);

		for event in events {
			assert!(conn.is_subscribed_to(&event));
		}
	}

	#[test]
	fn test_subscribe_duplicate_events_not_counted_twice() {
		let mut conn = create_test_connection();

		conn.subscribe(vec!["user.created".to_string()]);
		let change = conn.subscribe(vec!["user.created".to_string()]);

		assert_eq!(change.added, 0);
		assert_eq!(change.total, 1);
	}

	#[test]
	fn test_subscribe_mixed_new_and_existing() {
		let mut conn = create_test_connection();

		conn.subscribe(vec!["event1".to_string()]);
		let change = conn.subscribe(vec!["event1".to_string(), "event2".to_string(), "event3".to_string()]);

		assert_eq!(change.added, 2);
		assert_eq!(change.total, 3);
	}

	#[test]
	fn test_unsubscribe_from_subscribed_event() {
		let mut conn = create_test_connection();
		conn.subscribe(vec!["user.created".to_string()]);

		let change = conn.unsubscribe(vec!["user.created".to_string()]);

		assert_eq!(change.removed, 1);
		assert_eq!(change.total, 0);
		assert!(!conn.is_subscribed_to(&"user.created".to_string()));
	}

	#[test]
	fn test_unsubscribe_from_multiple_events() {
		let mut conn = create_test_connection();
		conn.subscribe(vec!["event1".to_string(), "event2".to_string(), "event3".to_string()]);

		let change = conn.unsubscribe(vec!["event1".to_string(), "event2".to_string()]);

		assert_eq!(change.removed, 2);
		assert_eq!(change.total, 1);
		assert!(conn.is_subscribed_to(&"event3".to_string()));
	}

	#[test]
	fn test_unsubscribe_from_non_subscribed_event() {
		let mut conn = create_test_connection();

		let change = conn.unsubscribe(vec!["nonexistent".to_string()]);

		assert_eq!(change.removed, 0);
		assert_eq!(change.total, 0);
	}

	#[test]
	fn test_mark_stale_transitions_from_active() {
		let mut conn = create_test_connection();

		conn.mark_stale("timeout".to_string());

		assert!(conn.is_stale());
		assert!(!conn.is_active());

		match conn.state {
			ConnectionState::Stale { ref reason, .. } => {
				assert_eq!(reason, "timeout");
			}
			_ => panic!("Expected Stale state"),
		}
	}

	#[test]
	fn test_mark_stale_preserves_last_ping() {
		let mut conn = create_test_connection();

		let original_ping = match conn.state {
			ConnectionState::Active { last_ping } => last_ping,
			_ => panic!("Expected Active state"),
		};

		sleep(Duration::from_millis(10));
		conn.mark_stale("timeout".to_string());

		let stale_ping = match conn.state {
			ConnectionState::Stale { last_ping, .. } => last_ping,
			_ => panic!("Expected Stale state"),
		};

		assert_eq!(original_ping, stale_ping);
	}

	#[test]
	fn test_mark_stale_does_not_affect_disconnected() {
		let mut conn = create_test_connection();
		conn.disconnect("closed".to_string());

		conn.mark_stale("timeout".to_string());

		assert!(conn.is_disconnected());
		assert!(!conn.is_stale());
	}

	#[test]
	fn test_disconnect_transitions_state() {
		let mut conn = create_test_connection();

		conn.disconnect("user requested".to_string());

		assert!(conn.is_disconnected());
		assert!(!conn.is_active());
		assert!(!conn.is_stale());
	}

	#[test]
	fn test_disconnect_captures_reason() {
		let mut conn = create_test_connection();
		let reason = "connection timeout".to_string();

		conn.disconnect(reason.clone());

		match conn.state {
			ConnectionState::Disconnected { reason: ref r, .. } => {
				assert_eq!(r, &reason);
			}
			_ => panic!("Expected Disconnected state"),
		}
	}

	#[test]
	fn test_disconnect_from_stale_state() {
		let mut conn = create_test_connection();
		conn.mark_stale("timeout".to_string());

		conn.disconnect("cleanup".to_string());

		assert!(conn.is_disconnected());
	}

	#[test]
	fn test_should_be_stale_returns_false_for_fresh_connection() {
		let conn = create_test_connection();

		let timeout = Duration::from_secs(60);
		assert!(!conn.should_be_stale(timeout));
	}

	#[test]
	fn test_should_be_stale_returns_true_after_timeout() {
		let mut conn = create_test_connection();

		// Manually set state with old timestamp
		let old_time = Instant::now() - Duration::from_secs(120);
		conn.state = ConnectionState::Active { last_ping: old_time };

		let timeout = Duration::from_secs(60);
		assert!(conn.should_be_stale(timeout));
	}

	#[test]
	fn test_should_be_stale_returns_false_for_stale_state() {
		let mut conn = create_test_connection();
		conn.mark_stale("timeout".to_string());

		let timeout = Duration::from_secs(60);
		assert!(!conn.should_be_stale(timeout));
	}

	#[test]
	fn test_should_be_stale_returns_false_for_disconnected_state() {
		let mut conn = create_test_connection();
		conn.disconnect("closed".to_string());

		let timeout = Duration::from_secs(60);
		assert!(!conn.should_be_stale(timeout));
	}

	#[test]
	fn test_is_subscribed_to_returns_false_for_unsubscribed() {
		let conn = create_test_connection();

		assert!(!conn.is_subscribed_to(&"user.created".to_string()));
	}

	#[test]
	fn test_get_duration_increases_over_time() {
		let conn = create_test_connection();

		let duration1 = conn.get_duration();
		sleep(Duration::from_millis(10));
		let duration2 = conn.get_duration();

		assert!(duration2 > duration1);
	}

	#[test]
	fn test_get_subscription_count_starts_at_zero() {
		let conn = create_test_connection();

		assert_eq!(conn.get_subscription_count(), 0);
	}

	#[test]
	fn test_get_subscription_count_updates_with_subscriptions() {
		let mut conn = create_test_connection();

		conn.subscribe(vec!["event1".to_string(), "event2".to_string()]);
		assert_eq!(conn.get_subscription_count(), 2);

		conn.subscribe(vec!["event3".to_string()]);
		assert_eq!(conn.get_subscription_count(), 3);

		conn.unsubscribe(vec!["event1".to_string()]);
		assert_eq!(conn.get_subscription_count(), 2);
	}

	#[test]
	fn test_get_subscriptions_returns_all_subscribed_events() {
		let mut conn = create_test_connection();

		let events = vec!["event1".to_string(), "event2".to_string(), "event3".to_string()];

		conn.subscribe(events.clone());
		let subscriptions = conn.get_subscriptions();

		assert_eq!(subscriptions.len(), 3);
		for event in events {
			assert!(subscriptions.contains(&event));
		}
	}

	#[test]
	fn test_get_subscriptions_returns_empty_set_initially() {
		let conn = create_test_connection();

		let subscriptions = conn.get_subscriptions();
		assert!(subscriptions.is_empty());
	}

	#[test]
	fn test_state_transitions_are_one_way() {
		let mut conn = create_test_connection();

		// Active -> Stale
		conn.mark_stale("timeout".to_string());
		assert!(conn.is_stale());

		// Stale -> Disconnected
		conn.disconnect("cleanup".to_string());
		assert!(conn.is_disconnected());

		// Cannot go back to stale
		conn.mark_stale("trying again".to_string());
		assert!(conn.is_disconnected());
	}

	#[test]
	fn test_connection_preserves_client_id() {
		let client_id = ClientId::new("unique-client-123");
		let addr = "192.168.1.1:9000".parse().unwrap();
		let conn = Connection::new(client_id.clone(), addr);

		assert_eq!(conn.client_id, client_id);
	}

	#[test]
	fn test_connection_preserves_source_addr() {
		let addr: SocketAddr = "10.0.0.1:3000".parse().unwrap();
		let conn = Connection::new(ClientId::new("test"), addr);

		assert_eq!(conn.source_addr, addr);
	}

	#[test]
	fn test_subscriptions_survive_state_transitions() {
		let mut conn = create_test_connection();

		conn.subscribe(vec!["event1".to_string(), "event2".to_string()]);
		assert_eq!(conn.get_subscription_count(), 2);

		conn.mark_stale("timeout".to_string());
		assert_eq!(conn.get_subscription_count(), 2);
		assert!(conn.is_subscribed_to(&"event1".to_string()));

		conn.disconnect("cleanup".to_string());
		assert_eq!(conn.get_subscription_count(), 2);
		assert!(conn.is_subscribed_to(&"event2".to_string()));
	}

	#[test]
	fn test_subscription_change_has_changes() {
		let mut conn = create_test_connection();

		let change1 = conn.subscribe(vec!["event1".to_string()]);
		assert!(change1.has_changes());

		let change2 = conn.subscribe(vec!["event1".to_string()]);
		assert!(!change2.has_changes());
	}

	#[test]
	fn test_subscription_change_net_change() {
		let mut conn = create_test_connection();

		let change = conn.subscribe(vec!["event1".to_string(), "event2".to_string()]);
		assert_eq!(change.net_change(), 2);

		let change = conn.unsubscribe(vec!["event1".to_string()]);
		assert_eq!(change.net_change(), -1);
	}

	#[test]
	fn test_multiple_clients_can_coexist() {
		let conn1 = create_connection_with_client("client1");
		let conn2 = create_connection_with_client("client2");

		assert_ne!(conn1.id, conn2.id);
		assert_ne!(conn1.client_id, conn2.client_id);
	}

	#[test]
	fn test_established_at_is_set_on_creation() {
		let before = Instant::now();
		let conn = create_test_connection();
		let after = Instant::now();

		assert!(conn.established_at >= before);
		assert!(conn.established_at <= after);
	}

	#[test]
	fn test_last_activity_matches_established_at_initially() {
		let conn = create_test_connection();

		// They should be very close (same instant in practice)
		let diff = if conn.last_activity > conn.established_at {
			conn.last_activity.duration_since(conn.established_at)
		} else {
			conn.established_at.duration_since(conn.last_activity)
		};

		assert!(diff < Duration::from_millis(1));
	}

	#[test]
	fn test_empty_subscription_operations() {
		let mut conn = create_test_connection();

		let change = conn.subscribe(Vec::<String>::new());
		assert_eq!(change.added, 0);
		assert_eq!(change.total, 0);

		let change = conn.unsubscribe(Vec::<String>::new());
		assert_eq!(change.removed, 0);
		assert_eq!(change.total, 0);
	}

	#[test]
	fn test_connection_debug_format() {
		let conn = create_test_connection();
		let debug_str = format!("{:?}", conn);

		assert!(debug_str.contains("Connection"));
		assert!(debug_str.contains("id"));
		assert!(debug_str.contains("client_id"));
	}
}
