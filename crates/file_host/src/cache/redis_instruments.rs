// Exported macro for recording cache operation metrics
#[macro_export]
macro_rules! record_cache_operation {
	($operation:expr, $result:expr, $duration:expr) => {
		if let Ok(counter) = &*$crate::CACHE_OPERATIONS_TOTAL {
			counter
				.with_label_values(&[
					$operation,
					match $result {
						Ok(_) => "success",
						Err(_) => "error",
					},
				])
				.inc();
		}

		if let Ok(histogram) = &*$crate::CACHE_OPERATION_DURATION {
			histogram.with_label_values(&[$operation]).observe($duration.as_secs_f64());
		}
	};
}

// Exported macro for recording cache hits/misses
#[macro_export]
macro_rules! record_cache_access {
	(hit, $operation:expr, $access_count:expr, $entry_age:expr) => {
		if let Ok(counter) = &*$crate::CACHE_HITS_TOTAL {
			counter.with_label_values(&[$operation]).inc();
		}

		if let Ok(histogram) = &*$crate::CACHE_ACCESS_COUNT {
			histogram.observe($access_count as f64);
		}

		if let Ok(gauge) = &*$crate::CACHE_ENTRY_AGE {
			gauge.set($entry_age as f64);
		}
	};
	(miss, $operation:expr) => {
		if let Ok(counter) = &*$crate::CACHE_MISSES_TOTAL {
			counter.inc();
		}
	};
}

// Exported macro for recording compression metrics
#[macro_export]
macro_rules! record_compression {
	($original_size:expr, $compressed_size:expr, $duration:expr) => {
		if let Ok(counter) = &*$crate::CACHE_COMPRESSIONS_TOTAL {
			counter.inc();
		}

		if let Ok(histogram) = &*$crate::CACHE_DATA_SIZE {
			histogram.observe($original_size as f64);
		}

		if let Ok(histogram) = &*$crate::CACHE_COMPRESSED_SIZE {
			histogram.observe($compressed_size as f64);
		}

		if let Ok(histogram) = &*$crate::CACHE_COMPRESSION_DURATION {
			histogram.observe($duration.as_secs_f64());
		}

		if $compressed_size > 0 {
			if let Ok(gauge) = &*$crate::CACHE_COMPRESSION_RATIO {
				let ratio = $original_size as f64 / $compressed_size as f64;
				gauge.set(ratio);
			}
		}
	};
}

// Exported macro for recording retry attempts
#[macro_export]
macro_rules! record_retry {
	($operation:expr, $attempt:expr, $error_type:expr) => {
		if let Ok(counter) = &*$crate::CACHE_RETRIES_TOTAL {
			counter.with_label_values(&[$operation, &$attempt.to_string()]).inc();
		}

		if let Ok(counter) = &*$crate::CACHE_ERRORS_TOTAL {
			counter.with_label_values(&[$operation, $error_type]).inc();
		}
	};
}
