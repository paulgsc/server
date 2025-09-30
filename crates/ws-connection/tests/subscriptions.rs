#[cfg(test)]
mod tests {
	use ws_connection::core::subscription::*;

	// ============================================================================
	// BASIC FUNCTIONALITY TESTS
	// ============================================================================

	#[test]
	fn test_new_creates_empty_manager() {
		let mgr: SubscriptionManager<String> = SubscriptionManager::new();
		assert!(mgr.is_empty());
		assert_eq!(mgr.count(), 0);
	}

	#[test]
	fn test_default_creates_empty_manager() {
		let mgr: SubscriptionManager<String> = SubscriptionManager::default();
		assert!(mgr.is_empty());
		assert_eq!(mgr.count(), 0);
	}

	#[test]
	fn test_with_subscriptions_creates_populated_manager() {
		let keys = vec!["event1".to_string(), "event2".to_string(), "event3".to_string()];
		let mgr = SubscriptionManager::with_subscriptions(keys.clone());

		assert_eq!(mgr.count(), 3);
		assert!(!mgr.is_empty());
		for key in keys {
			assert!(mgr.is_subscribed_to(&key));
		}
	}

	#[test]
	fn test_with_subscriptions_deduplicates() {
		let keys = vec!["event1".to_string(), "event1".to_string(), "event2".to_string()];
		let mgr = SubscriptionManager::with_subscriptions(keys);

		assert_eq!(mgr.count(), 2, "Duplicate keys should be deduplicated");
	}

	// ============================================================================
	// SUBSCRIBE TESTS
	// ============================================================================

	#[test]
	fn test_subscribe_single_key() {
		let mut mgr = SubscriptionManager::new();
		let change = mgr.subscribe(vec!["event1".to_string()]);

		assert_eq!(change.added, 1);
		assert_eq!(change.removed, 0);
		assert_eq!(change.total, 1);
		assert!(mgr.is_subscribed_to(&"event1".to_string()));
	}

	#[test]
	fn test_subscribe_multiple_keys() {
		let mut mgr = SubscriptionManager::new();
		let keys = vec!["event1".to_string(), "event2".to_string(), "event3".to_string()];
		let change = mgr.subscribe(keys.clone());

		assert_eq!(change.added, 3);
		assert_eq!(change.total, 3);
		for key in keys {
			assert!(mgr.is_subscribed_to(&key));
		}
	}

	#[test]
	fn test_subscribe_duplicate_keys_not_counted() {
		let mut mgr = SubscriptionManager::new();
		mgr.subscribe(vec!["event1".to_string()]);

		let change = mgr.subscribe(vec!["event1".to_string()]);
		assert_eq!(change.added, 0, "Re-subscribing should not add");
		assert_eq!(change.total, 1);
	}

	#[test]
	fn test_subscribe_mixed_new_and_existing() {
		let mut mgr = SubscriptionManager::new();
		mgr.subscribe(vec!["event1".to_string()]);

		let change = mgr.subscribe(vec!["event1".to_string(), "event2".to_string()]);
		assert_eq!(change.added, 1, "Only new key should be counted");
		assert_eq!(change.total, 2);
	}

	#[test]
	fn test_subscribe_empty_iterator() {
		let mut mgr = SubscriptionManager::new();
		let change = mgr.subscribe(Vec::<String>::new());

		assert_eq!(change.added, 0);
		assert_eq!(change.total, 0);
	}

	// ============================================================================
	// UNSUBSCRIBE TESTS
	// ============================================================================

	#[test]
	fn test_unsubscribe_existing_key() {
		let mut mgr = SubscriptionManager::with_subscriptions(vec!["event1".to_string()]);
		let change = mgr.unsubscribe(vec!["event1".to_string()]);

		assert_eq!(change.removed, 1);
		assert_eq!(change.added, 0);
		assert_eq!(change.total, 0);
		assert!(!mgr.is_subscribed_to(&"event1".to_string()));
	}

	#[test]
	fn test_unsubscribe_nonexistent_key() {
		let mut mgr = SubscriptionManager::new();
		let change = mgr.unsubscribe(vec!["nonexistent".to_string()]);

		assert_eq!(change.removed, 0);
		assert_eq!(change.total, 0);
	}

	#[test]
	fn test_unsubscribe_multiple_keys() {
		let keys = vec!["e1".to_string(), "e2".to_string(), "e3".to_string()];
		let mut mgr = SubscriptionManager::with_subscriptions(keys.clone());

		let change = mgr.unsubscribe(vec!["e1".to_string(), "e2".to_string()]);
		assert_eq!(change.removed, 2);
		assert_eq!(change.total, 1);
		assert!(mgr.is_subscribed_to(&"e3".to_string()));
	}

	#[test]
	fn test_unsubscribe_mixed_existing_nonexistent() {
		let mut mgr = SubscriptionManager::with_subscriptions(vec!["e1".to_string()]);
		let change = mgr.unsubscribe(vec!["e1".to_string(), "e2".to_string()]);

		assert_eq!(change.removed, 1, "Only existing key should be counted");
		assert_eq!(change.total, 0);
	}

	// ============================================================================
	// QUERY TESTS
	// ============================================================================

	#[test]
	fn test_is_subscribed_to_existing() {
		let mgr = SubscriptionManager::with_subscriptions(vec!["event1".to_string()]);
		assert!(mgr.is_subscribed_to(&"event1".to_string()));
	}

	#[test]
	fn test_is_subscribed_to_nonexistent() {
		let mgr = SubscriptionManager::<String>::new();
		assert!(!mgr.is_subscribed_to(&"event1".to_string()));
	}

	#[test]
	fn test_get_subscriptions_returns_correct_set() {
		let keys = vec!["e1".to_string(), "e2".to_string()];
		let mgr = SubscriptionManager::with_subscriptions(keys.clone());

		let subs = mgr.get_subscriptions();
		assert_eq!(subs.len(), 2);
		for key in keys {
			assert!(subs.contains(&key));
		}
	}

	#[test]
	fn test_get_subscriptions_is_reference() {
		let mgr = SubscriptionManager::with_subscriptions(vec!["e1".to_string()]);
		let subs1 = mgr.get_subscriptions();
		let subs2 = mgr.get_subscriptions();

		assert!(std::ptr::eq(subs1, subs2), "Should return same reference");
	}

	#[test]
	fn test_count_reflects_size() {
		let mut mgr = SubscriptionManager::new();
		assert_eq!(mgr.count(), 0);

		mgr.subscribe(vec!["e1".to_string(), "e2".to_string()]);
		assert_eq!(mgr.count(), 2);

		mgr.unsubscribe(vec!["e1".to_string()]);
		assert_eq!(mgr.count(), 1);
	}

	// ============================================================================
	// CLEAR TESTS
	// ============================================================================

	#[test]
	fn test_clear_removes_all_subscriptions() {
		let mut mgr = SubscriptionManager::with_subscriptions(vec!["e1".to_string(), "e2".to_string(), "e3".to_string()]);

		let change = mgr.clear();
		assert_eq!(change.removed, 3);
		assert_eq!(change.added, 0);
		assert_eq!(change.total, 0);
		assert!(mgr.is_empty());
	}

	#[test]
	fn test_clear_empty_manager() {
		let mut mgr = SubscriptionManager::<String>::new();
		let change = mgr.clear();

		assert_eq!(change.removed, 0);
		assert_eq!(change.total, 0);
	}

	// ============================================================================
	// SET_SUBSCRIPTIONS TESTS
	// ============================================================================

	#[test]
	fn test_set_subscriptions_replaces_all() {
		let mut mgr = SubscriptionManager::with_subscriptions(vec!["old1".to_string(), "old2".to_string()]);

		let change = mgr.set_subscriptions(vec!["new1".to_string(), "new2".to_string()]);
		assert_eq!(change.total, 2);

		assert!(!mgr.is_subscribed_to(&"old1".to_string()));
		assert!(!mgr.is_subscribed_to(&"old2".to_string()));
		assert!(mgr.is_subscribed_to(&"new1".to_string()));
		assert!(mgr.is_subscribed_to(&"new2".to_string()));
	}

	#[test]
	fn test_set_subscriptions_to_empty() {
		let mut mgr = SubscriptionManager::with_subscriptions(vec!["e1".to_string()]);
		let change = mgr.set_subscriptions(Vec::<String>::new());

		assert_eq!(change.total, 0);
		assert!(mgr.is_empty());
	}

	#[test]
	fn test_set_subscriptions_identical_no_changes() {
		let keys = vec!["e1".to_string(), "e2".to_string()];
		let mut mgr = SubscriptionManager::with_subscriptions(keys.clone());

		let change = mgr.set_subscriptions(keys);
		assert_eq!(change.added, 0);
		assert_eq!(change.removed, 0);
		assert_eq!(change.total, 2);
	}

	#[test]
	fn test_set_subscriptions_deduplicates() {
		let mut mgr = SubscriptionManager::new();
		let change = mgr.set_subscriptions(vec!["e1".to_string(), "e1".to_string(), "e2".to_string()]);

		assert_eq!(change.total, 2);
		assert_eq!(mgr.count(), 2);
	}

	// ============================================================================
	// SUBSCRIPTION_CHANGE TESTS
	// ============================================================================

	#[test]
	fn test_net_change_positive() {
		let change = SubscriptionChange { added: 5, removed: 2, total: 10 };
		assert_eq!(change.net_change(), 3);
	}

	#[test]
	fn test_net_change_negative() {
		let change = SubscriptionChange { added: 2, removed: 5, total: 5 };
		assert_eq!(change.net_change(), -3);
	}

	#[test]
	fn test_net_change_zero() {
		let change = SubscriptionChange { added: 3, removed: 3, total: 10 };
		assert_eq!(change.net_change(), 0);
	}

	#[test]
	fn test_has_changes_true_when_added() {
		let change = SubscriptionChange { added: 1, removed: 0, total: 1 };
		assert!(change.has_changes());
	}

	#[test]
	fn test_has_changes_true_when_removed() {
		let change = SubscriptionChange { added: 0, removed: 1, total: 0 };
		assert!(change.has_changes());
	}

	#[test]
	fn test_has_changes_false_when_no_changes() {
		let change = SubscriptionChange { added: 0, removed: 0, total: 5 };
		assert!(!change.has_changes());
	}

	// ============================================================================
	// CLONE TESTS
	// ============================================================================

	#[test]
	fn test_manager_clone_creates_independent_copy() {
		let mgr1 = SubscriptionManager::with_subscriptions(vec!["e1".to_string()]);
		let mut mgr2 = mgr1.clone();

		mgr2.subscribe(vec!["e2".to_string()]);

		assert_eq!(mgr1.count(), 1);
		assert_eq!(mgr2.count(), 2);
		assert!(!mgr1.is_subscribed_to(&"e2".to_string()));
		assert!(mgr2.is_subscribed_to(&"e2".to_string()));
	}

	#[test]
	fn test_subscription_change_clone() {
		let change1 = SubscriptionChange { added: 1, removed: 2, total: 3 };
		let change2 = change1.clone();

		assert_eq!(change2.added, 1);
		assert_eq!(change2.removed, 2);
		assert_eq!(change2.total, 3);
	}

	// ============================================================================
	// GENERIC TYPE TESTS
	// ============================================================================

	#[test]
	fn test_with_integer_keys() {
		let mut mgr = SubscriptionManager::<u32>::new();
		mgr.subscribe(vec![1, 2, 3]);

		assert!(mgr.is_subscribed_to(&1));
		assert!(mgr.is_subscribed_to(&2));
		assert_eq!(mgr.count(), 3);
	}

	#[test]
	fn test_with_custom_enum_keys() {
		#[derive(Debug, Clone, PartialEq, Eq, Hash)]
		enum EventType {
			UserJoined,
			UserLeft,
			MessageSent,
		}

		let mut mgr = SubscriptionManager::<EventType>::new();
		mgr.subscribe(vec![EventType::UserJoined, EventType::MessageSent]);

		assert!(mgr.is_subscribed_to(&EventType::UserJoined));
		assert!(!mgr.is_subscribed_to(&EventType::UserLeft));
		assert_eq!(mgr.count(), 2);
	}

	#[test]
	fn test_with_tuple_keys() {
		let mut mgr = SubscriptionManager::<(String, u32)>::new();
		mgr.subscribe(vec![("event".to_string(), 1), ("event".to_string(), 2)]);

		assert!(mgr.is_subscribed_to(&("event".to_string(), 1)));
		assert_eq!(mgr.count(), 2);
	}

	// ============================================================================
	// EDGE CASES & STRESS TESTS
	// ============================================================================

	#[test]
	fn test_large_number_of_subscriptions() {
		let mut mgr = SubscriptionManager::new();
		let keys: Vec<String> = (0..10_000).map(|i| format!("event_{}", i)).collect();

		let change = mgr.subscribe(keys.clone());
		assert_eq!(change.added, 10_000);
		assert_eq!(mgr.count(), 10_000);

		// Verify random samples
		assert!(mgr.is_subscribed_to(&"event_0".to_string()));
		assert!(mgr.is_subscribed_to(&"event_5000".to_string()));
		assert!(mgr.is_subscribed_to(&"event_9999".to_string()));
	}

	#[test]
	fn test_rapid_subscribe_unsubscribe_cycles() {
		let mut mgr = SubscriptionManager::new();
		let key = "event".to_string();

		for _ in 0..1000 {
			mgr.subscribe(vec![key.clone()]);
			mgr.unsubscribe(vec![key.clone()]);
		}

		assert!(mgr.is_empty());
	}

	#[test]
	fn test_subscribe_after_clear() {
		let mut mgr = SubscriptionManager::with_subscriptions(vec!["e1".to_string()]);
		mgr.clear();
		mgr.subscribe(vec!["e2".to_string()]);

		assert_eq!(mgr.count(), 1);
		assert!(!mgr.is_subscribed_to(&"e1".to_string()));
		assert!(mgr.is_subscribed_to(&"e2".to_string()));
	}

	#[test]
	fn test_empty_string_key() {
		let mut mgr = SubscriptionManager::new();
		mgr.subscribe(vec!["".to_string()]);

		assert!(mgr.is_subscribed_to(&"".to_string()));
		assert_eq!(mgr.count(), 1);
	}

	#[test]
	fn test_unicode_keys() {
		let mut mgr = SubscriptionManager::new();
		let keys = vec!["🎉".to_string(), "你好".to_string(), "🚀event".to_string()];
		mgr.subscribe(keys.clone());

		for key in keys {
			assert!(mgr.is_subscribed_to(&key));
		}
	}

	#[test]
	fn test_very_long_key() {
		let mut mgr = SubscriptionManager::new();
		let long_key = "a".repeat(10_000);
		mgr.subscribe(vec![long_key.clone()]);

		assert!(mgr.is_subscribed_to(&long_key));
	}

	// ============================================================================
	// INTEGRATION / WORKFLOW TESTS
	// ============================================================================

	#[test]
	fn test_typical_connection_lifecycle() {
		let mut mgr = SubscriptionManager::new();

		// Initial subscription
		mgr.subscribe(vec!["chat:room1".to_string(), "chat:room2".to_string()]);
		assert_eq!(mgr.count(), 2);

		// Switch rooms
		mgr.unsubscribe(vec!["chat:room1".to_string()]);
		mgr.subscribe(vec!["chat:room3".to_string()]);
		assert_eq!(mgr.count(), 2);
		assert!(mgr.is_subscribed_to(&"chat:room2".to_string()));
		assert!(mgr.is_subscribed_to(&"chat:room3".to_string()));

		// Disconnect
		mgr.clear();
		assert!(mgr.is_empty());
	}

	#[test]
	fn test_bulk_operations_preserve_invariants() {
		let mut mgr = SubscriptionManager::new();

		// Subscribe to 100 events
		let keys: Vec<String> = (0..100).map(|i| format!("e{}", i)).collect();
		mgr.subscribe(keys.clone());

		// Unsubscribe from half
		let to_remove: Vec<String> = (0..50).map(|i| format!("e{}", i)).collect();
		mgr.unsubscribe(to_remove);

		assert_eq!(mgr.count(), 50);
		assert!(!mgr.is_subscribed_to(&"e0".to_string()));
		assert!(mgr.is_subscribed_to(&"e50".to_string()));
	}

	// ============================================================================
	// DEBUG & DISPLAY TESTS
	// ============================================================================

	#[test]
	fn test_debug_format_works() {
		let mgr = SubscriptionManager::with_subscriptions(vec!["e1".to_string()]);
		let debug_str = format!("{:?}", mgr);
		assert!(debug_str.contains("SubscriptionManager"));
	}

	#[test]
	fn test_subscription_change_debug() {
		let change = SubscriptionChange { added: 1, removed: 2, total: 3 };
		let debug_str = format!("{:?}", change);
		assert!(debug_str.contains("added"));
		assert!(debug_str.contains("removed"));
		assert!(debug_str.contains("total"));
	}
}

// ============================================================================
// PROPERTY-BASED TESTS (using standard library only)
// ============================================================================

#[cfg(test)]
mod property_tests {
	use ws_connection::core::subscription::*;

	#[test]
	fn property_count_never_negative() {
		let mut mgr = SubscriptionManager::<u32>::new();
		for i in 0..100 {
			mgr.subscribe(vec![i]);
			assert!(mgr.count() as i32 >= 0);
		}
	}

	#[test]
	fn property_subscribe_idempotent() {
		let mut mgr = SubscriptionManager::new();
		let key = "event".to_string();

		mgr.subscribe(vec![key.clone()]);
		let count_after_first = mgr.count();

		mgr.subscribe(vec![key.clone()]);
		let count_after_second = mgr.count();

		assert_eq!(count_after_first, count_after_second);
	}

	#[test]
	fn property_unsubscribe_inverse_of_subscribe() {
		let mut mgr = SubscriptionManager::new();
		let key = "event".to_string();

		mgr.subscribe(vec![key.clone()]);
		mgr.unsubscribe(vec![key.clone()]);

		assert!(mgr.is_empty());
	}

	#[test]
	fn property_set_equals_clear_then_subscribe() {
		let keys = vec!["e1".to_string(), "e2".to_string()];

		let mut mgr1 = SubscriptionManager::with_subscriptions(vec!["old".to_string()]);
		mgr1.set_subscriptions(keys.clone());

		let mut mgr2 = SubscriptionManager::with_subscriptions(vec!["old".to_string()]);
		mgr2.clear();
		mgr2.subscribe(keys);

		assert_eq!(mgr1.get_subscriptions(), mgr2.get_subscriptions());
	}
}
