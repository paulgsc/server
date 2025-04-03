use std::collections::HashMap;
use std::hash::Hash;
use std::ptr;

struct LruEntry<K, V> {
	key: K,
	value: V,
	prev: *mut LruEntry<K, V>,
	next: *mut LruEntry<K, V>,
}

struct LruCache<K, V>
where
	K: Eq + Hash + Clone,
{
	map: HashMap<K, *mut LruEntry<K, V>>,
	head: *mut LruEntry<K, V>,
	tail: *mut LruEntry<K, V>,
	capacity: usize,
}

impl<K: Eq + Hash + Clone, V> LruCache<K, V> {
	fn new(capacity: usize) -> Self {
		LruCache {
			map: HashMap::with_capacity(capacity),
			head: ptr::null_mut(),
			tail: ptr::null_mut(),
			capacity,
		}
	}

	fn get(&mut self, key: &K) -> Option<&V> {
		if let Some(&entry_ptr) = self.map.get(key) {
			unsafe {
				self.detach(entry_ptr);
				self.attach_front(entry_ptr);
				return Some(&(*entry_ptr).value);
			}
		}
		None
	}

	fn put(&mut self, key: K, value: V) {
		if let Some(&entry_ptr) = self.map.get(&key) {
			unsafe {
				(*entry_ptr).value = value;
				self.detach(entry_ptr);
				self.attach_front(entry_ptr);
				return;
			}
		}

		let new_entry = Box::new(LruEntry {
			key: key.clone(),
			value,
			prev: ptr::null_mut(),
			next: ptr::null_mut(),
		});
		let new_entry_ptr = Box::into_raw(new_entry);

		self.map.insert(key, new_entry_ptr);
		unsafe {
			self.attach_front(new_entry_ptr);
		}

		if self.map.len() > self.capacity {
			unsafe {
				if !self.tail.is_null() {
					let tail_entry = Box::from_raw(self.tail);
					self.map.remove(&tail_entry.key);
					self.tail = tail_entry.prev;
					if !self.tail.is_null() {
						(*self.tail).next = ptr::null_mut();
					}
				}
			}
		}
	}

	unsafe fn detach(&mut self, entry_ptr: *mut LruEntry<K, V>) {
		if (*entry_ptr).prev.is_null() {
			self.head = (*entry_ptr).next;
		} else {
			(*(*entry_ptr).prev).next = (*entry_ptr).next;
		}

		if (*entry_ptr).next.is_null() {
			self.tail = (*entry_ptr).prev;
		} else {
			(*(*entry_ptr).next).prev = (*entry_ptr).prev;
		}
	}

	unsafe fn attach_front(&mut self, entry_ptr: *mut LruEntry<K, V>) {
		(*entry_ptr).prev = ptr::null_mut();
		(*entry_ptr).next = self.head;

		if !self.head.is_null() {
			(*self.head).prev = entry_ptr;
		}

		self.head = entry_ptr;
		if self.tail.is_null() {
			self.tail = entry_ptr;
		}
	}
}

impl<K: Eq + Hash + Clone, V> Drop for LruCache<K, V> {
	fn drop(&mut self) {
		unsafe {
			let mut current = self.head;
			while !current.is_null() {
				let next = (*current).next;
				drop(Box::from_raw(current));
				current = next;
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_lru_cache_new() {
		let cache: LruCache<i32, i32> = LruCache::new(2);
		assert_eq!(cache.map.len(), 0);
	}

	#[test]
	fn test_lru_cache_put_and_get() {
		let mut cache = LruCache::new(2);
		cache.put(1, 10);
		cache.put(2, 20);

		assert_eq!(cache.get(&1), Some(&10));
		assert_eq!(cache.get(&2), Some(&20));
		assert_eq!(cache.get(&3), None);
	}

	#[test]
	fn test_lru_cache_eviction() {
		let mut cache = LruCache::new(2);
		cache.put(1, 10);
		cache.put(2, 20);
		cache.put(3, 30); // This should evict key 1

		assert_eq!(cache.get(&1), None);
		assert_eq!(cache.get(&2), Some(&20));
		assert_eq!(cache.get(&3), Some(&30));
	}

	#[test]
	fn test_lru_cache_update_existing_key() {
		let mut cache = LruCache::new(2);
		cache.put(1, 10);
		cache.put(2, 20);
		cache.put(1, 100); // Update value for key 1

		assert_eq!(cache.get(&1), Some(&100));
		assert_eq!(cache.get(&2), Some(&20));
	}

	#[test]
	fn test_lru_cache_reorder_on_access() {
		let mut cache = LruCache::new(2);
		cache.put(1, 10);
		cache.put(2, 20);
		cache.get(&1); // Access key 1 to mark it as recently used
		cache.put(3, 30); // This should evict key 2

		assert_eq!(cache.get(&1), Some(&10));
		assert_eq!(cache.get(&2), None);
		assert_eq!(cache.get(&3), Some(&30));
	}
}
