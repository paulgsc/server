#[derive(Debug)]
pub struct TransportMetrics {
	connection_id: String,
	connections_established: Counter,
	connections_failed: Counter,
	messages_sent: Counter,
	messages_received: Counter,
	bytes_sent: Counter,
	bytes_received: Counter,
	message_send_duration: Histogram,
	connection_duration: Histogram,
	active_connections: IntGauge,
	send_queue_size: Gauge,
	receive_queue_size: Gauge,
}

impl TransportMetrics {
	pub fn new(connection_id: &str) -> Result<Self, Box<dyn std::error::Error>> {
		let labels = &[("connection_id", connection_id)];

		Ok(Self {
			connection_id: connection_id.to_string(),
			connections_established: register_counter!("transport_connections_established_total", "Total number of established connections", labels)?,
			connections_failed: register_counter!("transport_connections_failed_total", "Total number of failed connections", labels)?,
			messages_sent: register_counter!("transport_messages_sent_total", "Total number of messages sent", labels)?,
			messages_received: register_counter!("transport_messages_received_total", "Total number of messages received", labels)?,
			bytes_sent: register_counter!("transport_bytes_sent_total", "Total bytes sent", labels)?,
			bytes_received: register_counter!("transport_bytes_received_total", "Total bytes received", labels)?,
			message_send_duration: register_histogram!("transport_message_send_duration_seconds", "Time taken to send messages", labels)?,
			connection_duration: register_histogram!("transport_connection_duration_seconds", "Duration of connections", labels)?,
			active_connections: register_int_gauge!("transport_active_connections", "Number of active connections", labels)?,
			send_queue_size: register_gauge!("transport_send_queue_size", "Current send queue size", labels)?,
			receive_queue_size: register_gauge!("transport_receive_queue_size", "Current receive queue size", labels)?,
		})
	}

	pub fn record_connection_established(&self) {
		self.connections_established.inc();
		self.active_connections.inc();
	}

	pub fn record_connection_failed(&self) {
		self.connections_failed.inc();
	}

	pub fn record_connection_closed(&self) {
		self.active_connections.dec();
	}

	pub fn record_message_sent(&self, bytes: usize, duration: Duration) {
		self.messages_sent.inc();
		self.bytes_sent.inc_by(bytes as u64);
		self.message_send_duration.observe(duration.as_secs_f64());
	}

	pub fn record_message_received(&self, bytes: usize) {
		self.messages_received.inc();
		self.bytes_received.inc_by(bytes as u64);
	}

	pub fn update_queue_sizes(&self, send_size: usize, receive_size: usize) {
		self.send_queue_size.set(send_size as f64);
		self.receive_queue_size.set(receive_size as f64);
	}
}
