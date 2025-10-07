pub struct NatsSubscriber<K: EventKey> {
	subscribers: Vec<Subscriber>,
	_phantom: std::marker::PhantomData<K>,
}

impl<K: EventKey> NatsSubscriber<K> {
	pub async fn next(&mut self) -> Option<NatsEvent<K>> {
		if self.subscribers.is_empty() {
			return None;
		}

		use futures::StreamExt;

		let mut combined_stream = futures::stream::select_all(self.subscribers.iter_mut().map(|s| s.by_ref()));

		while let Some(msg) = combined_stream.next().await {
			match serde_json::from_slice::<NatsEvent<K>>(&msg.payload) {
				Ok(event) => return Some(event),
				Err(e) => {
					error!("Failed to deserialize event: {}", e);
					continue;
				}
			}
		}

		None
	}

	pub async fn unsubscribe(self) -> Result<()> {
		for mut sub in self.subscribers {
			sub.unsubscribe().await.map_err(|_| NatsError::ChannelError)?;
		}
		Ok(())
	}
}
