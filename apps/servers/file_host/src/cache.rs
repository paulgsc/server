pub use some_cache::{CacheConfig, CacheEntry, CacheError, CacheStore, DedupCache};

/// How many concurrent fetches the dedup cache tracks; older ones are evicted
/// past it. A source constant (#617), as `MAX_IN_FLIGHT` was by default.
pub const MAX_IN_FLIGHT: u64 = 256;

/// How long a cached entry lives, in seconds: the cache's default and the
/// now-playing event's. A source constant (#617), as `CACHE_TTL` was by
/// default.
pub const CACHE_TTL: u64 = 600;
