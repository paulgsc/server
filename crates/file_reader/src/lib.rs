pub mod config;
pub mod core;

use async_trait::async_trait;
use futures::StreamExt;
use object_store::path::Path;
use object_store::{GetResult, GetResultPayload, ObjectStore};
use std::io::{Read, Seek};
use std::sync::Arc; // For async traits

#[async_trait]
pub trait HtmlProcessor {
	async fn process_html_content(&self, html_content: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

pub struct ChunkedReader {
	pub store: Arc<dyn ObjectStore>,
}

impl ChunkedReader {
	pub fn new(store: Arc<dyn ObjectStore>) -> Self {
		ChunkedReader { store }
	}

	pub async fn read_large_file<P: HtmlProcessor + Send + Sync>(&self, path: &str, processor: &P) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
		let object_path = Path::from(path);
		let object: GetResult = self.store.get_opts(&object_path, Default::default()).await?;

		let html_content = match object.payload {
			GetResultPayload::File(mut file, _path) => {
				let len = object.range.end - object.range.start;
				file.seek(std::io::SeekFrom::Start(object.range.start as u64))?;
				let mut buffer = vec![0; len];
				file.read_exact(&mut buffer)?;
				String::from_utf8_lossy(&buffer).to_string()
			}
			GetResultPayload::Stream(stream) => {
				let mut buffer = Vec::new();
				let mut stream = stream.map(|result| result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
				while let Some(chunk) = stream.next().await {
					buffer.extend_from_slice(&chunk?);
				}
				String::from_utf8_lossy(&buffer).to_string()
			}
		};

		processor.process_html_content(&html_content).await
	}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use object_store::{GetOptions, ObjectMeta};

    // Mock implementation of HtmlProcessor for testing
    struct MockHtmlProcessor {
        pub call_count: Arc<Mutex<usize>>,  // Shared state to track number of calls
    }

    #[async_trait]
    impl HtmlProcessor for MockHtmlProcessor {
        async fn process_html_content(&self, _html_content: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            Ok(())
        }
    }

    // Mock ObjectStore implementation for testing
    struct MockObjectStore;

    #[async_trait]
    impl ObjectStore for MockObjectStore {
        async fn get_opts(&self, _path: &Path, _options: GetOptions) -> Result<GetResult, object_store::Error> {
            // Mock file payload with some dummy HTML content
            Ok(GetResult {
                meta: ObjectMeta {
                    location: Path::from("dummy/path"),
                    last_modified: chrono::Utc::now(),
                    size: 100,
                    e_tag: None,
                },
                range: object_store::Range { start: 0, end: 11 }, // Mock range
                payload: GetResultPayload::File(
                    std::io::Cursor::new(b"<html></html>".to_vec()), // Dummy content
                    Path::from("dummy/path")
                ),
            })
        }

        // The rest of the ObjectStore trait's required methods would be mocked here as needed
        // For brevity, these aren't necessary for this example test.
    }

    #[tokio::test]
    async fn test_read_large_file() {
        let store = Arc::new(MockObjectStore);
        let chunked_reader = ChunkedReader::new(store);

        let call_count = Arc::new(Mutex::new(0));
        let processor = MockHtmlProcessor { call_count: call_count.clone() };

        // Execute the method and verify that the HTML content processing method was called
        chunked_reader.read_large_file("dummy/path", &processor).await.unwrap();

        // Verify that process_html_content was called once
        assert_eq!(*call_count.lock().unwrap(), 1);
    }
}
