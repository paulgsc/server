use crate::messages::ObsEvent;
use async_broadcast::Receiver;
use axum::extract::ws::{Message, WebSocket};
use futures_util::{sink::SinkExt, stream::StreamExt};

pub struct WebSocketHandler {
	obs_receiver: Receiver<ObsEvent>,
}

impl WebSocketHandler {
	pub fn new(obs_receiver: Receiver<ObsEvent>) -> Self {
		Self { obs_receiver }
	}

	pub async fn handle(mut self, socket: WebSocket) {
		let (mut sender, mut receiver) = socket.split();

		// Task to send OBS events to WebSocket client
		let send_task = tokio::spawn(async move {
			while let Ok(event) = self.obs_receiver.recv().await {
				if let Ok(json) = serde_json::to_string(&event) {
					if sender.send(Message::Text(json)).await.is_err() {
						break;
					}
				}
			}
		});

		// Task to handle incoming WebSocket messages
		let recv_task = tokio::spawn(async move {
			while let Some(msg) = receiver.next().await {
				match msg {
					Ok(Message::Close(_)) => break,
					Ok(_) => {} // Handle other message types if needed
					Err(_) => break,
				}
			}
		});

		let _ = tokio::join!(send_task, recv_task);
	}
}
