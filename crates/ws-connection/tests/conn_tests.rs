//! Tests for the connection layer.
//!
//! The crate splits a connection into three pieces, and the modules below
//! mirror that split: `Connection` is immutable metadata, `ConnectionState`
//! is the mutable half the actor owns, and `ConnectionHandle` is the only way
//! the two are reached from outside the actor task.

#[cfg(test)]
mod connection_metadata {
	use std::net::SocketAddr;
	use std::thread::sleep;
	use std::time::{Duration, Instant};
	use ws_connection::core::conn::Connection;
	use ws_connection::types::ClientId;

	fn test_connection() -> Connection {
		let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
		Connection::new(ClientId::new("test-client"), addr)
	}

	#[test]
	fn new_records_the_client_and_address_it_was_given() {
		let client_id = ClientId::new("test-client");
		let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
		let conn = Connection::new(client_id.clone(), addr);

		assert_eq!(conn.client_id, client_id);
		assert_eq!(conn.source_addr, addr);
	}

	#[test]
	fn each_connection_gets_a_unique_id() {
		let conn1 = test_connection();
		let conn2 = test_connection();

		assert_ne!(conn1.id, conn2.id);
	}

	#[test]
	fn distinct_clients_stay_distinct() {
		let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
		let conn1 = Connection::new(ClientId::new("client1"), addr);
		let conn2 = Connection::new(ClientId::new("client2"), addr);

		assert_ne!(conn1.id, conn2.id);
		assert_ne!(conn1.client_id, conn2.client_id);
	}

	#[test]
	fn established_at_is_stamped_during_construction() {
		let before = Instant::now();
		let conn = test_connection();
		let after = Instant::now();

		assert!(conn.established_at >= before);
		assert!(conn.established_at <= after);
	}

	#[test]
	fn get_duration_grows_with_wall_clock_time() {
		let conn = test_connection();

		let first = conn.get_duration();
		sleep(Duration::from_millis(10));
		let second = conn.get_duration();

		assert!(second > first);
	}

	#[test]
	fn cloning_preserves_identity() {
		let conn = test_connection();
		let clone = conn.clone();

		assert_eq!(conn.id, clone.id);
		assert_eq!(conn.client_id, clone.client_id);
		assert_eq!(conn.source_addr, clone.source_addr);
	}

	#[test]
	fn debug_output_names_the_struct_and_its_fields() {
		let conn = test_connection();
		let debug_str = format!("{conn:?}");

		assert!(debug_str.contains("Connection"));
		assert!(debug_str.contains("id"));
		assert!(debug_str.contains("client_id"));
	}
}

#[cfg(test)]
mod connection_state {
	use std::thread::sleep;
	use std::time::Duration as StdDuration;
	use tokio::time::Duration;
	use ws_connection::actor::ConnectionState;

	#[test]
	fn new_state_is_active_and_neither_stale_nor_disconnected() {
		let state = ConnectionState::new();

		assert!(state.is_active);
		assert!(!state.is_stale);
		assert!(state.stale_reason.is_none());
		assert!(state.disconnect_reason.is_none());
	}

	#[test]
	fn default_matches_new() {
		let state = ConnectionState::default();

		assert!(state.is_active);
		assert!(!state.is_stale);
	}

	#[test]
	fn record_activity_advances_the_timestamp() {
		let mut state = ConnectionState::new();
		let before = state.last_activity;

		sleep(StdDuration::from_millis(10));
		state.record_activity();

		assert!(state.last_activity > before);
	}

	#[test]
	fn should_be_stale_is_false_while_activity_is_recent() {
		let state = ConnectionState::new();

		assert!(!state.should_be_stale(Duration::from_secs(60)));
	}

	#[test]
	fn should_be_stale_is_true_once_the_timeout_elapses() {
		let state = ConnectionState::new();

		sleep(StdDuration::from_millis(20));

		assert!(state.should_be_stale(Duration::from_millis(1)));
	}

	#[test]
	fn recording_activity_resets_the_staleness_clock() {
		let mut state = ConnectionState::new();
		sleep(StdDuration::from_millis(20));
		assert!(state.should_be_stale(Duration::from_millis(1)));

		state.record_activity();

		assert!(!state.should_be_stale(Duration::from_secs(60)));
	}

	#[test]
	fn should_be_stale_is_false_for_a_connection_already_marked_stale() {
		let mut state = ConnectionState::new();
		state.mark_stale("timeout".to_string());

		sleep(StdDuration::from_millis(20));

		assert!(!state.should_be_stale(Duration::from_millis(1)));
	}

	#[test]
	fn mark_stale_deactivates_and_captures_the_reason() {
		let mut state = ConnectionState::new();

		state.mark_stale("timeout".to_string());

		assert!(!state.is_active);
		assert!(state.is_stale);
		assert_eq!(state.stale_reason.as_deref(), Some("timeout"));
	}

	#[test]
	fn mark_stale_keeps_the_first_reason() {
		let mut state = ConnectionState::new();
		state.mark_stale("timeout".to_string());

		state.mark_stale("something else".to_string());

		assert_eq!(state.stale_reason.as_deref(), Some("timeout"));
	}

	#[test]
	fn disconnect_captures_the_reason_and_clears_active() {
		let mut state = ConnectionState::new();

		state.disconnect("user requested".to_string());

		assert!(!state.is_active);
		assert!(!state.is_stale);
		assert_eq!(state.disconnect_reason.as_deref(), Some("user requested"));
	}

	#[test]
	fn disconnect_supersedes_stale() {
		let mut state = ConnectionState::new();
		state.mark_stale("timeout".to_string());

		state.disconnect("cleanup".to_string());

		assert!(!state.is_stale);
		assert_eq!(state.disconnect_reason.as_deref(), Some("cleanup"));
	}

	#[test]
	fn a_disconnected_connection_cannot_go_back_to_stale() {
		let mut state = ConnectionState::new();
		state.disconnect("closed".to_string());

		state.mark_stale("trying again".to_string());

		assert!(!state.is_stale);
		assert!(state.stale_reason.is_none());
		assert_eq!(state.disconnect_reason.as_deref(), Some("closed"));
	}

	#[test]
	fn as_str_describes_each_stage_of_the_lifecycle() {
		let mut state = ConnectionState::new();
		assert_eq!(state.as_str(), "active");

		state.mark_stale("timeout".to_string());
		assert_eq!(state.as_str(), "inactive, stale(timeout)");

		state.disconnect("cleanup".to_string());
		assert_eq!(state.as_str(), "inactive, disconnected(cleanup)");
	}

	#[test]
	fn display_matches_as_str() {
		let mut state = ConnectionState::new();
		state.mark_stale("timeout".to_string());

		assert_eq!(state.to_string(), state.as_str());
	}
}

#[cfg(test)]
mod connection_actor {
	use std::net::SocketAddr;
	use std::time::Duration;
	use tokio_util::sync::CancellationToken;
	use ws_connection::actor::ConnectionHandle;
	use ws_connection::core::conn::Connection;
	use ws_connection::types::ClientId;

	#[derive(Debug, Clone, PartialEq, Eq, Hash)]
	enum UserEvent {
		Created,
		Updated,
		Deleted,
	}

	/// Spawn an actor and hand back the only thing callers ever hold: its
	/// handle. The `CancellationToken` is kept alive by the returned guard so
	/// the parent token is not dropped out from under the child.
	fn spawn_handle<K>() -> (ConnectionHandle<K>, CancellationToken)
	where
		K: ws_connection::core::subscription::EventKey,
	{
		let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
		let connection = Connection::new(ClientId::new("test-client"), addr);
		let parent = CancellationToken::new();
		let (handle, actor, _token) = ConnectionHandle::new(connection, 16, &parent);

		tokio::spawn(actor.run());

		(handle, parent)
	}

	#[tokio::test]
	async fn a_fresh_actor_reports_no_subscriptions() {
		let (handle, _parent) = spawn_handle::<String>();

		let subs = handle.get_subscriptions().await.unwrap();

		assert!(subs.is_empty());
	}

	#[tokio::test]
	async fn the_handle_caches_the_immutable_connection_metadata() {
		let (handle, _parent) = spawn_handle::<String>();

		assert_eq!(handle.connection.client_id, ClientId::new("test-client"));
		assert_eq!(handle.connection.source_addr, "127.0.0.1:8080".parse::<SocketAddr>().unwrap());
	}

	#[tokio::test]
	async fn subscribe_is_visible_to_a_later_query() {
		let (handle, _parent) = spawn_handle::<String>();

		handle.subscribe(vec!["user.created".to_string(), "user.updated".to_string()]).await.unwrap();

		let subs = handle.get_subscriptions().await.unwrap();
		assert_eq!(subs.len(), 2);
		assert!(handle.is_subscribed_to("user.created".to_string()).await.unwrap());
		assert!(!handle.is_subscribed_to("user.deleted".to_string()).await.unwrap());
	}

	#[tokio::test]
	async fn subscribe_deduplicates_across_calls() {
		let (handle, _parent) = spawn_handle::<String>();

		handle.subscribe(vec!["event1".to_string()]).await.unwrap();
		handle.subscribe(vec!["event1".to_string(), "event2".to_string()]).await.unwrap();

		let subs = handle.get_subscriptions().await.unwrap();
		assert_eq!(subs.len(), 2);
	}

	#[tokio::test]
	async fn unsubscribe_removes_only_the_named_keys() {
		let (handle, _parent) = spawn_handle::<String>();
		handle.subscribe(vec!["e1".to_string(), "e2".to_string(), "e3".to_string()]).await.unwrap();

		handle.unsubscribe(vec!["e1".to_string(), "e2".to_string()]).await.unwrap();

		let subs = handle.get_subscriptions().await.unwrap();
		assert_eq!(subs.len(), 1);
		assert!(handle.is_subscribed_to("e3".to_string()).await.unwrap());
	}

	#[tokio::test]
	async fn unsubscribing_an_unknown_key_is_a_no_op() {
		let (handle, _parent) = spawn_handle::<String>();
		handle.subscribe(vec!["e1".to_string()]).await.unwrap();

		handle.unsubscribe(vec!["nonexistent".to_string()]).await.unwrap();

		assert!(handle.is_subscribed_to("e1".to_string()).await.unwrap());
	}

	#[tokio::test]
	async fn enum_keys_work_the_same_as_string_keys() {
		let (handle, _parent) = spawn_handle::<UserEvent>();

		handle.subscribe(vec![UserEvent::Created, UserEvent::Updated]).await.unwrap();

		assert!(handle.is_subscribed_to(UserEvent::Created).await.unwrap());
		assert!(handle.is_subscribed_to(UserEvent::Updated).await.unwrap());
		assert!(!handle.is_subscribed_to(UserEvent::Deleted).await.unwrap());
	}

	#[tokio::test]
	async fn record_activity_holds_off_a_staleness_check() {
		let (handle, _parent) = spawn_handle::<String>();

		handle.record_activity().await.unwrap();
		handle.check_stale(Duration::from_secs(60)).await.unwrap();

		let state = handle.get_state().await.unwrap();
		assert!(state.is_active);
		assert!(!state.is_stale);
	}

	#[tokio::test]
	async fn check_stale_marks_the_connection_once_the_timeout_elapses() {
		let (handle, _parent) = spawn_handle::<String>();

		tokio::time::sleep(Duration::from_millis(20)).await;
		handle.check_stale(Duration::from_millis(1)).await.unwrap();

		let state = handle.get_state().await.unwrap();
		assert!(state.is_stale);
		assert!(!state.is_active);
		assert_eq!(state.stale_reason.as_deref(), Some("timeout"));
	}

	#[tokio::test]
	async fn mark_stale_records_the_caller_supplied_reason() {
		let (handle, _parent) = spawn_handle::<String>();

		handle.mark_stale("idle".to_string()).await.unwrap();

		let state = handle.get_state().await.unwrap();
		assert!(state.is_stale);
		assert_eq!(state.stale_reason.as_deref(), Some("idle"));
	}

	#[tokio::test]
	async fn subscriptions_survive_being_marked_stale() {
		let (handle, _parent) = spawn_handle::<String>();
		handle.subscribe(vec!["e1".to_string(), "e2".to_string()]).await.unwrap();

		handle.mark_stale("timeout".to_string()).await.unwrap();

		let subs = handle.get_subscriptions().await.unwrap();
		assert_eq!(subs.len(), 2);
	}

	#[tokio::test]
	async fn disconnect_stops_the_actor() {
		let (handle, _parent) = spawn_handle::<String>();

		handle.disconnect("user requested".to_string()).await.unwrap();

		// `Disconnect` breaks the actor loop, so the command channel closes and
		// every later request fails rather than hanging.
		let outcome = wait_for_actor_exit(&handle).await;
		assert!(outcome.is_err(), "actor should stop accepting commands after disconnect");
	}

	#[tokio::test]
	async fn shutdown_stops_the_actor() {
		let (handle, _parent) = spawn_handle::<String>();

		handle.shutdown().await.unwrap();

		let outcome = wait_for_actor_exit(&handle).await;
		assert!(outcome.is_err(), "actor should stop accepting commands after shutdown");
	}

	#[tokio::test]
	async fn cloned_handles_address_the_same_actor() {
		let (handle, _parent) = spawn_handle::<String>();
		let clone = handle.clone();

		clone.subscribe(vec!["shared".to_string()]).await.unwrap();

		assert!(handle.is_subscribed_to("shared".to_string()).await.unwrap());
	}

	/// Poll the actor until it stops answering. The actor exits asynchronously
	/// after `Disconnect`/`Shutdown`, so a single immediate query can still land
	/// in the channel buffer and succeed; this retries until the channel closes.
	async fn wait_for_actor_exit<K>(handle: &ConnectionHandle<K>) -> ws_connection::actor::error::Result<()>
	where
		K: ws_connection::core::subscription::EventKey,
	{
		for _ in 0..100 {
			handle.record_activity().await?;
			tokio::time::sleep(Duration::from_millis(5)).await;
		}
		Ok(())
	}
}
