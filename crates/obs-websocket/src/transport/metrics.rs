#[derive(Debug)]
pub struct TransportMetrics {
	// Connection metrics
	pub connections_total: Counter,
	pub connections_active: IntGauge,
	pub connections_failed: Counter,
	pub connection_duration: Histogram,

	// Message metrics
	pub messages_sent_total: Counter,
	pub messages_received_total: Counter,
	pub messages_dropped_total: Counter,
	pub message_send_duration: Histogram,
	pub message_size_bytes: Histogram,

	// Frame metrics
	pub frames_sent_total: Counter,
	pub frames_received_total: Counter,
	pub frame_size_bytes: Histogram,

	// Buffer metrics
	pub send_queue_size: IntGauge,
	pub receive_queue_size: IntGauge,
	pub send_buffer_utilization: Gauge,
	pub receive_buffer_utilization: Gauge,

	// Flow control metrics
	pub send_credits_available: IntGauge,
	pub receive_credits_available: IntGauge,
	pub backpressure_events_total: Counter,
	pub flow_control_violations_total: Counter,

	// Keepalive metrics
	pub pings_sent_total: Counter,
	pub pongs_received_total: Counter,
	pub ping_rtt_seconds: Histogram,
	pub keepalive_failures_total: Counter,

	// Error metrics
	pub errors_total: Counter,
	pub timeouts_total: Counter,
	pub reconnections_total: Counter,
	pub protocol_errors_total: Counter,

	// Performance metrics
	pub cpu_usage_percent: Gauge,
	pub memory_usage_bytes: IntGauge,
	pub bandwidth_utilization_percent: Gauge,
}

impl TransportMetrics {
	pub fn new(connection_id: &str) -> Result<Self, prometheus::Error> {
		let labels = &[("connection_id", connection_id)];

		Ok(Self {
			connections_total: register_counter!("websocket_connections_total", "Total number of WebSocket connections attempted", labels)?,
			connections_active: register_int_gauge!("websocket_connections_active", "Number of currently active WebSocket connections", labels)?,
			connections_failed: register_counter!("websocket_connections_failed_total", "Total number of failed WebSocket connections", labels)?,
			connection_duration: register_histogram!(
				"websocket_connection_duration_seconds",
				"Duration of WebSocket connections",
				vec![0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 300.0, 1800.0],
				labels
			)?,
			messages_sent_total: register_counter!("websocket_messages_sent_total", "Total number of messages sent", labels)?,
			messages_received_total: register_counter!("websocket_messages_received_total", "Total number of messages received", labels)?,
			messages_dropped_total: register_counter!("websocket_messages_dropped_total", "Total number of messages dropped due to buffer overflow", labels)?,
			message_send_duration: register_histogram!(
				"websocket_message_send_duration_seconds",
				"Time taken to send messages",
				vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0],
				labels
			)?,
			message_size_bytes: register_histogram!(
				"websocket_message_size_bytes",
				"Size of WebSocket messages",
				vec![64.0, 256.0, 1024.0, 4096.0, 16384.0, 65536.0, 262144.0, 1048576.0],
				labels
			)?,
			frames_sent_total: register_counter!("websocket_frames_sent_total", "Total number of frames sent", labels)?,
			frames_received_total: register_counter!("websocket_frames_received_total", "Total number of frames received", labels)?,
			frame_size_bytes: register_histogram!(
				"websocket_frame_size_bytes",
				"Size of WebSocket frames",
				vec![64.0, 256.0, 1024.0, 4096.0, 16384.0, 65536.0],
				labels
			)?,
			send_queue_size: register_int_gauge!("websocket_send_queue_size", "Current size of send queue", labels)?,
			receive_queue_size: register_int_gauge!("websocket_receive_queue_size", "Current size of receive queue", labels)?,
			send_buffer_utilization: register_gauge!("websocket_send_buffer_utilization", "Send buffer utilization (0.0 to 1.0)", labels)?,
			receive_buffer_utilization: register_gauge!("websocket_receive_buffer_utilization", "Receive buffer utilization (0.0 to 1.0)", labels)?,
			send_credits_available: register_int_gauge!("websocket_send_credits_available", "Available send credits for flow control", labels)?,
			receive_credits_available: register_int_gauge!("websocket_receive_credits_available", "Available receive credits for flow control", labels)?,
			backpressure_events_total: register_counter!("websocket_backpressure_events_total", "Total number of backpressure events", labels)?,
			flow_control_violations_total: register_counter!("websocket_flow_control_violations_total", "Total number of flow control violations", labels)?,
			pings_sent_total: register_counter!("websocket_pings_sent_total", "Total number of ping frames sent", labels)?,
			pongs_received_total: register_counter!("websocket_pongs_received_total", "Total number of pong frames received", labels)?,
			ping_rtt_seconds: register_histogram!(
				"websocket_ping_rtt_seconds",
				"Round-trip time for ping/pong",
				vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0],
				labels
			)?,
			keepalive_failures_total: register_counter!("websocket_keepalive_failures_total", "Total number of keepalive failures", labels)?,
			errors_total: register_counter!("websocket_errors_total", "Total number of errors", labels)?,
			timeouts_total: register_counter!("websocket_timeouts_total", "Total number of timeout events", labels)?,
			reconnections_total: register_counter!("websocket_reconnections_total", "Total number of reconnection attempts", labels)?,
			protocol_errors_total: register_counter!("websocket_protocol_errors_total", "Total number of protocol errors", labels)?,
			cpu_usage_percent: register_gauge!("websocket_cpu_usage_percent", "CPU usage percentage", labels)?,
			memory_usage_bytes: register_int_gauge!("websocket_memory_usage_bytes", "Memory usage in bytes", labels)?,
			bandwidth_utilization_percent: register_gauge!("websocket_bandwidth_utilization_percent", "Bandwidth utilization percentage", labels)?,
		})
	}

	// Add missing methods for complete metrics recording
	pub fn record_message_received(&self, size: usize) {
		self.messages_received_total.inc();
		self.message_size_bytes.observe(size as f64);
	}

	pub fn record_frame_sent(&self, size: usize) {
		self.frames_sent_total.inc();
		self.frame_size_bytes.observe(size as f64);
	}

	pub fn record_frame_received(&self, size: usize) {
		self.frames_received_total.inc();
		self.frame_size_bytes.observe(size as f64);
	}

	pub fn record_ping_sent(&self) {
		self.pings_sent_total.inc();
	}

	pub fn record_pong_received(&self, rtt: Duration) {
		self.pongs_received_total.inc();
		self.ping_rtt_seconds.observe(rtt.as_secs_f64());
	}

	pub fn record_backpressure_event(&self) {
		self.backpressure_events_total.inc();
	}

	pub fn record_flow_control_violation(&self) {
		self.flow_control_violations_total.inc();
	}

	pub fn record_keepalive_failure(&self) {
		self.keepalive_failures_total.inc();
	}

	pub fn record_reconnection(&self) {
		self.reconnections_total.inc();
	}

	pub fn update_queue_sizes(&self, send_queue: usize, receive_queue: usize) {
		self.send_queue_size.set(send_queue as i64);
		self.receive_queue_size.set(receive_queue as i64);
	}

	pub fn update_buffer_utilization(&self, send_util: f64, receive_util: f64) {
		self.send_buffer_utilization.set(send_util);
		self.receive_buffer_utilization.set(receive_util);
	}

	pub fn update_flow_control_credits(&self, send_credits: i32, receive_credits: i32) {
		self.send_credits_available.set(send_credits as i64);
		self.receive_credits_available.set(receive_credits as i64);
	}

	pub fn update_resource_usage(&self, cpu_percent: f64, memory_bytes: u64, bandwidth_percent: f64) {
		self.cpu_usage_percent.set(cpu_percent);
		self.memory_usage_bytes.set(memory_bytes as i64);
		self.bandwidth_utilization_percent.set(bandwidth_percent);
	}

	pub fn record_connection_established(&self) {
		self.connections_total.inc();
		self.connections_active.inc();
	}

	pub fn record_connection_closed(&self, duration: Duration) {
		self.connections_active.dec();
		self.connection_duration.observe(duration.as_secs_f64());
	}

	pub fn record_message_sent(&self, size: usize, duration: Duration) {
		self.messages_sent_total.inc();
		self.message_size_bytes.observe(size as f64);
		self.message_send_duration.observe(duration.as_secs_f64());
	}

	pub fn record_error(&self, error: &TransportError) {
		self.errors_total.inc();

		match error {
			TransportError::Timeout { .. } => self.timeouts_total.inc(),
			TransportError::Protocol { .. } => self.protocol_errors_total.inc(),
			_ => {}
		}
	}
}
