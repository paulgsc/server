use std::time::Duration;
use tokio::sync::{Mutex, Notify};

pub(crate) struct Debouncer {
    bump: Notify,
    serial: Mutex<Option<usize>>,
    timeout: Duration,
}

impl Debouncer {
    pub fn new(timeout: Duration) -> Self {
        Self {
            bump: Notify::new(),
            serial: Mutex::new(None),
            timeout,
        }
    }

    pub fn bump(&self) {
        self.bump.notify_one();
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

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    use crate::debouncer::Debouncer;

    #[tokio::test]
    async fn test_debouncer() {
        let debouncer = Arc::new(Debouncer::new(Duration::from_millis(10)));
        let debouncer_copy = debouncer.clone();
        let handle = tokio::task::spawn(async move {
            debouncer_copy.debounce().await;
        });
        for _ in 0..10 {
            // assert that we can continue bumping it past the original timeout
            tokio::time::sleep(Duration::from_millis(2)).await;
            assert!(debouncer.bump());
        }
        let start = Instant::now();
        handle.await.unwrap();
        let end = Instant::now();
        // give some wiggle room to account for race conditions, but assert that we
        // didn't immediately complete after the last bump
        assert!(end - start > Duration::from_millis(5));
        // we shouldn't be able to bump it after it's run out it's timeout
        assert!(!debouncer.bump());
    }
}
