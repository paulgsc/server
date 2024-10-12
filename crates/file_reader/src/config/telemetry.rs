use chrono::{DateTime, Utc};
use log::info;

pub struct TelemetryLog {
    message: String,
    timestamp: DateTime<Utc>,
    expiry: DateTime<Utc>,
}

impl TelemetryLog {
    pub fn new(message: &str, duration: chrono::Duration) -> Self {
        let now = Utc::now();
        Self {
            message: message.to_string(),
            timestamp: now,
            expiry: now + duration,
        }
    }

    pub fn log(&self) {
        info!("{}: {}", self.timestamp, self.message);
    }
}
