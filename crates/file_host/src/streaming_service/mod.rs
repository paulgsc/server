use crate::{cache::lru_cache::LruCache, Config};
use bytes::Bytes;
use futures::Stream;
use futures::TryStreamExt;
use std::io;
use std::path::Path;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use tokio::sync::{mpsc, Semaphore};
use tokio_stream::wrappers::ReceiverStream;

pub struct StreamingService {
	// Add fields for your storage provider
	// e.g. gcs_client: storage::Client,
	max_chunks: usize,
	chunk_size: usize,
	semaphore: Arc<Semaphore>,
	lru_cache: Arc<tokio::sync::Mutex<LruCache<String, Vec<u8>>>>,
}

impl StreamingService {
	pub fn new(config: Arc<Config>) -> Self {
		let max_chunks = config.max_chunks;
		let chunk_size = config.chunk_size;

		StreamingService {
			max_chunks,
			chunk_size,
			semaphore: Arc::new(Semaphore::new(max_chunks)),
			lru_cache: Arc::new(tokio::sync::Mutex::new(LruCache::new(config.cache_ttl.try_into().unwrap()))),
		}
	}

	/// Stream a local file (for testing purposes)
	pub async fn stream_local_file<P: AsRef<Path>>(&self, path: P) -> io::Result<impl Stream<Item = io::Result<Bytes>> + Send + 'static> {
		let path_str = path.as_ref().to_string_lossy().to_string(); // Convert path to String for caching
		let mut cache = self.lru_cache.lock().await;

		if let Some(cached_data) = cache.get(&path_str) {
			// Serve from cache
			println!("Serving from cache: {}", path_str); // Add log
			let stream = self.create_memory_stream(cached_data.clone());
			Ok(stream)
		} else {
			// Read from file, cache, and serve
			let file = File::open(path).await?;
			let mut reader = BufReader::with_capacity(self.chunk_size, file);
			let mut data = Vec::new();
			let mut buffer = vec![0u8; self.chunk_size];

			loop {
				let bytes_read = reader.read(&mut buffer).await?;
				if bytes_read == 0 {
					break;
				}
				data.extend_from_slice(&buffer[..bytes_read]);
			}
			cache.put(path_str.clone(), data.clone()); // Cache the data
			let stream = self.create_memory_stream(data);
			Ok(stream)
		}
	}

	fn create_memory_stream(&self, data: Vec<u8>) -> impl Stream<Item = io::Result<Bytes>> + Send + 'static {
		let chunk_size = self.chunk_size;
		let (tx, rx) = mpsc::channel(self.max_chunks);
		let semaphore = self.semaphore.clone();

		tokio::spawn(async move {
			let mut start = 0;
			while start < data.len() {
				let end = std::cmp::min(start + chunk_size, data.len());
				let chunk = Bytes::copy_from_slice(&data[start..end]);

				let permit = match semaphore.acquire().await {
					Ok(permit) => permit,
					Err(_) => break,
				};

				if tx.send(Ok(chunk)).await.is_err() {
					break;
				}
				start = end;
				drop(permit);
			}
		});
		ReceiverStream::new(rx)
	}

	/// Creates a backpressure-aware stream from any AsyncRead source
	pub fn create_stream_with_backpressure<R>(&self, reader: R) -> impl Stream<Item = io::Result<Bytes>> + Send + 'static
	where
		R: AsyncRead + Unpin + Send + 'static,
	{
		// Create a channel for our chunks
		let (tx, rx) = mpsc::channel(self.max_chunks);
		let semaphore_producer = self.semaphore.clone();
		let semaphore_consumer = self.semaphore.clone();
		let chunk_size = self.chunk_size;

		// Spawn a task to read from the source and send chunks through the channel
		tokio::spawn(async move {
			let mut reader = reader;
			let mut buffer = vec![0u8; chunk_size].into_boxed_slice();

			loop {
				// Acquire a permit from the semaphore (implements backpressure)
				let _ = match semaphore_producer.acquire().await {
					Ok(permit) => permit,
					Err(_) => {
						// Semaphore closed, exit loop.  Important for shutdown.
						break;
					}
				};

				// Read a chunk
				let bytes_read = match reader.read(&mut buffer).await {
					Ok(0) => break, // EOF
					Ok(n) => n,
					Err(e) => {
						if tx.send(Err(e)).await.is_err() {
							// Error sending error.  Receiver is gone.
							break;
						}
						break;
					}
				};

				// Send the chunk through the channel
				let chunk = Bytes::copy_from_slice(&buffer[..bytes_read]);
				if tx.send(Ok(chunk)).await.is_err() {
					break; // Receiver dropped
				}
				// Permit is dropped here
			}
			// Drop tx to close channel.
		});

		// Create a stream from the receiver that releases semaphore permits
		ReceiverStream::new(rx) // Use the tokio_stream version.
			.inspect_ok(move |_| {
				semaphore_consumer.add_permits(1);
			})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use futures::StreamExt;
	use std::io::Write;
	use tempfile::NamedTempFile;
	use tokio_test::block_on;

	fn create_test_file(size: usize) -> io::Result<NamedTempFile> {
		let mut file = NamedTempFile::new()?;
		let data = vec![0u8; size];
		file.write_all(&data)?;
		file.flush()?;
		Ok(file)
	}

	#[tokio::test]
	async fn test_stream_with_backpressure() {
		// Create a 1MB test file
		let file = create_test_file(1024 * 1024).unwrap();
		let path = file.path();

		// Create streaming service with small chunks and limited concurrency
		let service = StreamingService::new(Some(2), Some(16 * 1024));

		// Stream the file
		let mut stream = service.stream_local_file(path).await.unwrap();

		let mut total_bytes = 0;

		// Process the stream
		while let Some(chunk_result) = stream.next().await {
			match chunk_result {
				Ok(chunk) => {
					total_bytes += chunk.len();
					// Simulate slow consumer by adding delay
					sleep(Duration::from_millis(10)).await;
				}
				Err(e) => panic!("Error reading chunk: {:?}", e),
			}
		}

		assert_eq!(total_bytes, 1024 * 1024);
	}
}
