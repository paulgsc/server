// Copyright (c) 2024 Vercel, Inc
// //
// // Permission is hereby granted, free of charge, to any person obtaining a copy of this software
// and associated documentation files (the "Software"), to deal in the Software without
// restriction, including without limitation the rights to use, copy, modify, merge, publish,
// distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
// //
// // The above copyright notice and this permission notice shall be included in all copies or
// substantial portions of the Software.
// //
// // THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
// BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
// NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
// DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//
//

use std::{fmt::Debug, sync::Mutex, time::Duration};
use tokio::{select, sync, time::Instant};
use tracing::trace;

const DEFAULT_DEBOUNCE_TIMEOUT: Duration = Duration::from_millis(10);

pub struct Debouncer {
	bump: sync::Notify,
	serial: Mutex<Option<usize>>,
	timeout: Duration,
}

impl Default for Debouncer {
	fn default() -> Self {
		Self::new(DEFAULT_DEBOUNCE_TIMEOUT)
	}
}

impl Debug for Debouncer {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let serial = { self.serial.lock().expect("lock is valid") };
		f.debug_struct("Debouncer").field("is_pending", &serial.is_some()).field("timeout", &self.timeout).finish()
	}
}

impl Debouncer {
	pub fn new(timeout: Duration) -> Self {
		let bump = sync::Notify::new();
		let serial = Mutex::new(Some(0));
		Self { bump, serial, timeout }
	}

	pub fn bump(&self) -> bool {
		let mut serial = self.serial.lock().expect("lock is valid");
		match *serial {
			None => false,
			Some(previous) => {
				*serial = Some(previous + 1);
				self.bump.notify_one();
				true
			}
		}
	}

	pub async fn debounce(&self) {
		let mut serial = { self.serial.lock().expect("lock is valid").expect("only this thread sets the value to None") };
		let mut deadline = Instant::now() + self.timeout;
		loop {
			let timeout = tokio::time::sleep_until(deadline);
			select! {
					_ = self.bump.notified() => {
							trace!("debouncer notified");
							let current_serial = self.serial.lock().expect("lock is valid").expect("only this thread sets the value to None");
							if current_serial != serial {
									serial = current_serial;
									deadline = Instant::now() + self.timeout;
							}
					}
					_ = timeout => {
							let mut current_serial_opt = self.serial.lock().expect("lock is valid");
							let current_serial = current_serial_opt.expect("only this thread sets the value to None");
							if current_serial == serial {
									*current_serial_opt = None;
									return;
							} else {
									serial = current_serial;
									deadline = Instant::now() + self.timeout;
							}
					}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::Arc;
	use std::time::{Duration, Instant};
	use tokio::task;

	#[tokio::test]
	async fn test_debouncer_bump() {
		let debouncer = Arc::new(Debouncer::new(Duration::from_millis(10)));
		let debouncer_clone = debouncer.clone();
		let handle = task::spawn(async move {
			debouncer_clone.debounce().await;
		});

		assert!(debouncer.bump());
		tokio::time::sleep(Duration::from_millis(5)).await;

		let start = Instant::now();
		assert!(debouncer.bump());
		tokio::time::sleep(Duration::from_millis(15)).await;
		let elapsed = start.elapsed();

		assert!(elapsed.as_millis() >= 10);
		handle.await.unwrap();
		assert!(!debouncer.bump());
	}

	#[tokio::test]
	async fn test_debouncer_timeout() {
		let debouncer = Arc::new(Debouncer::new(Duration::from_millis(10)));
		let debouncer_clone = debouncer.clone();
		let handle = task::spawn(async move {
			debouncer_clone.debounce().await;
		});

		tokio::time::sleep(Duration::from_millis(20)).await;
		assert!(!debouncer.bump());
		handle.await.unwrap();
	}

	#[tokio::test]
	async fn test_debouncer_multiple_bumps() {
		let debouncer = Arc::new(Debouncer::new(Duration::from_millis(10)));
		let debouncer_clone = debouncer.clone();
		let handle = task::spawn(async move {
			debouncer_clone.debounce().await;
		});

		for _ in 0..5 {
			assert!(debouncer.bump());
			tokio::time::sleep(Duration::from_millis(2)).await;
		}

		tokio::time::sleep(Duration::from_millis(15)).await;
		assert!(!debouncer.bump());
		handle.await.unwrap();
	}
}
