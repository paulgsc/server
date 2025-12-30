use std::time::Duration;
use tokio::time::sleep;

/// Retry an async operation up to `max_attempts` times with exponential backoff.
/// - `operation` is a closure that returns a `Result<T, E>`
/// - `max_attempts` is the total number of tries (including the first one)
/// - `base_delay` is the initial backoff delay
/// - `factor` is the multiplier for each retry (e.g. 2 for exponential)
///
/// Returns `Ok(T)` on success or `Err(E)` from the last attempt.
pub async fn retry_async<F, Fut, T, E>(mut operation: F, max_attempts: usize, base_delay: Duration, factor: u32) -> Result<T, E>
where
	F: FnMut() -> Fut,
	Fut: std::future::Future<Output = Result<T, E>>,
{
	let mut attempt = 0;

	loop {
		attempt += 1;
		match operation().await {
			Ok(result) => return Ok(result),
			Err(err) if attempt >= max_attempts => return Err(err),
			Err(_) => {
				let backoff = base_delay * factor.pow((attempt - 1) as u32);
				sleep(backoff).await;
			}
		}
	}
}
