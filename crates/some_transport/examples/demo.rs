/// Example usage function
pub async fn example_usage() -> Result<(), Box<dyn std::error::Error>> {
	// Create transport configuration
	let config = TransportConfig {
		keepalive: KeepaliveConfig {
			enabled: true,
			interval: Duration::from_secs(30),
			timeout: Duration::from_secs(10),
			idle_timeout: Duration::from_secs(300),
			max_failures: 3,
		},
		flow_control: FlowControlConfig {
			send_buffer_size: 1000,
			receive_buffer_size: 1000,
			backpressure_threshold: 100,
		},
		buffer: BufferConfig {
			send_queue_size: 1000,
			receive_queue_size: 1000,
			frame_buffer_size: 64 * 1024,
			message_buffer_size: 1024 * 1024,
			max_send_queue_size: 10000,
			max_receive_queue_size: 10000,
			queue_policy: QueuePolicy::DropOldest,
		},
	};

	// Create the transport system
	let (actor, command_tx, mut event_rx) = create_websocket_transport(config);

	// Spawn the actor
	let actor_handle = tokio::spawn(async move {
		actor.run().await;
	});

	// Connect to an endpoint
	let endpoint = Endpoint {
		host: "echo.websocket.org".to_string(),
		port: 80,
		path: "/".to_string(),
		secure: false,
	};

	let (response_tx, response_rx) = oneshot::channel();
	command_tx
		.send(TransportCommand::Connect {
			endpoint,
			config: TransportConfig {
				keepalive: KeepaliveConfig {
					enabled: true,
					interval: Duration::from_secs(30),
					timeout: Duration::from_secs(10),
					idle_timeout: Duration::from_secs(300),
					max_failures: 3,
				},
				flow_control: FlowControlConfig {
					send_buffer_size: 1000,
					receive_buffer_size: 1000,
					backpressure_threshold: 100,
				},
				buffer: BufferConfig {
					send_queue_size: 1000,
					receive_queue_size: 1000,
					frame_buffer_size: 64 * 1024,
					message_buffer_size: 1024 * 1024,
					max_send_queue_size: 10000,
					max_receive_queue_size: 10000,
					queue_policy: QueuePolicy::DropOldest,
				},
			},
			respond_to: response_tx,
		})
		.await?;

	// Wait for connection result
	match response_rx.await? {
		Ok(connection_info) => {
			println!("Connected: {:?}", connection_info);
		}
		Err(e) => {
			println!("Connection failed: {}", e);
			return Err(e.into());
		}
	}

	// Listen for events
	tokio::spawn(async move {
		while let Ok(event) = event_rx.recv().await {
			println!("Transport event: {:?}", event);
		}
	});

	// Send a test message
	let (send_tx, send_rx) = oneshot::channel();
	command_tx
		.send(TransportCommand::SendMessage {
			message: OutgoingMessage {
				content: MessageContent::Text("Hello, WebSocket!".to_string()),
				priority: MessagePriority::Normal,
				timeout: Some(Duration::from_secs(30)),
				correlation_id: None,
				response_channel: None,
			},
			respond_to: send_tx,
		})
		.await?;

	match send_rx.await? {
		Ok(message_id) => {
			println!("Message sent with ID: {:?}", message_id);
		}
		Err(e) => {
			println!("Failed to send message: {}", e);
		}
	}

	// Graceful shutdown
	tokio::time::sleep(Duration::from_secs(5)).await;

	let (disconnect_tx, disconnect_rx) = oneshot::channel();
	command_tx
		.send(TransportCommand::Disconnect {
			close_code: CloseCode::Normal,
			reason: Some("Example finished".to_string()),
			respond_to: disconnect_tx,
		})
		.await?;

	disconnect_rx.await??;
	actor_handle.await?;

	Ok(())
}
