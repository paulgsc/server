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

// src/debouncer.rs
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fmt::Debug, sync::Mutex, time::Duration};
use tokio::{sync::Notify, time::Instant};

pub struct Debouncer {
	bump: Notify,
	serial: Mutex<Option<usize>>,
	timeout: Duration,
	is_active: AtomicBool,
}

impl Debug for Debouncer {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let serial = { self.serial.lock().expect("lock is valid") };
		f.debug_struct("Debouncer").field("is_pending", &serial.is_some()).field("timeout", &self.timeout).finish()
	}
}

impl Debouncer {
	pub fn new(timeout: Duration) -> Self {
		Self {
			bump: Notify::new(),
			serial: Mutex::new(None),
			timeout,
			is_active: AtomicBool::new(true),
		}
	}

	pub fn bump(&self) -> bool {
		if self.is_active.load(Ordering::SeqCst) {
			self.bump.notify_one();
			true
		} else {
			false
		}
	}

	pub async fn debounce(&self) {
		let mut last_bump = Instant::now();
		loop {
			tokio::select! {
					_ = self.bump.notified() => {
							last_bump = Instant::now();
					}
					_ = tokio::time::sleep(self.timeout) => {
							if last_bump.elapsed() >= self.timeout {
									break;
							}
					}
			}
		}
		self.is_active.store(false, Ordering::SeqCst);
	}

	pub async fn wait(&self) {
		let mut serial = self.serial.lock().await;
		let next = serial.map(|s| s + 1).unwrap_or(0);
		*serial = Some(next);
		drop(serial);

		tokio::select! {
				_ = self.bump.notified() => return,
				_ = tokio::time::sleep(self.timeout) => {},
		}

		let serial = self.serial.lock().await;
		if *serial != Some(next) {
			return;
		}
		*serial = None;
	}
}

// src/lib.rs
// ... (previous code remains the same)

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
			debouncer_clone.wait().await;
		});

		debouncer.bump();
		tokio::time::sleep(Duration::from_millis(5)).await;

		let start = Instant::now();
		debouncer.bump();
		tokio::time::sleep(Duration::from_millis(15)).await;
		let elapsed = start.elapsed();

		assert!(elapsed.as_millis() >= 10);
		handle.await.unwrap();
	}

	#[tokio::test]
	async fn test_debouncer_wait_timeout() {
		let debouncer = Arc::new(Debouncer::new(Duration::from_millis(10)));
		let debouncer_clone = debouncer.clone();
		let handle = task::spawn(async move {
			debouncer_clone.wait().await;
		});

		tokio::time::sleep(Duration::from_millis(20)).await;
		assert!(handle.is_finished());
	}

	#[tokio::test]
	async fn test_debouncer_multiple_bumps() {
		let debouncer = Arc::new(Debouncer::new(Duration::from_millis(10)));
		let debouncer_clone = debouncer.clone();
		let handle = task::spawn(async move {
			debouncer_clone.wait().await;
		});

		for _ in 0..5 {
			debouncer.bump();
			tokio::time::sleep(Duration::from_millis(2)).await;
		}

		tokio::time::sleep(Duration::from_millis(15)).await;
		assert!(handle.is_finished());
	}

	#[tokio::test]
	async fn test_debouncer() {
		let debouncer = Arc::new(Debouncer::new(Duration::from_millis(10)));
		let debouncer_copy = debouncer.clone();
		let handle = tokio::task::spawn(async move {
			debouncer_copy.debounce().await;
		});

		for _ in 0..10 {
			tokio::time::sleep(Duration::from_millis(2)).await;
			assert!(debouncer.bump());
		}

		let start = Instant::now();
		handle.await.unwrap();
		let end = Instant::now();

		assert!(end - start > Duration::from_millis(5));
		assert!(!debouncer.bump());
	}
}
