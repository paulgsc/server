//! Each request a `Client` sends is recorded once, under closed labels only.

use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use outbound::{Caller, RECEIVED_METRIC, REQUESTS_METRIC, SENT_METRIC};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A loopback server that answers each connection with `status` and `body`.
async fn answering(status: &'static str, body: &'static str) -> std::io::Result<String> {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
	let addr = listener.local_addr()?;
	tokio::spawn(async move {
		while let Ok((mut stream, _)) = listener.accept().await {
			let mut buf = [0_u8; 4096];
			let _ = stream.read(&mut buf).await;
			let answer = "HTTP/1.1 ".to_owned() + status + "\r\ncontent-length: " + &body.len().to_string() + "\r\nconnection: close\r\n\r\n" + body;
			let _ = stream.write_all(answer.as_bytes()).await;
		}
	});
	Ok("http://".to_owned() + &addr.to_string() + "/some/path?q=private")
}

/// A label set, sorted.
type Labels = Vec<(String, String)>;

/// Every recorded counter as (name, sorted labels, value).
fn counters(recorder: &DebuggingRecorder) -> Vec<(String, Labels, u64)> {
	let mut out: Vec<_> = recorder
		.snapshotter()
		.snapshot()
		.into_vec()
		.into_iter()
		.filter_map(|(key, _, _, value)| match value {
			DebugValue::Counter(count) => {
				let mut labels: Vec<_> = key.key().labels().map(|label| (label.key().to_owned(), label.value().to_owned())).collect();
				labels.sort();
				Some((key.key().name().to_owned(), labels, count))
			}
			_ => None,
		})
		.collect();
	out.sort();
	out
}

fn labels(caller: &str, host: &str, outcome: Option<&str>) -> Labels {
	let mut labels = vec![(String::from("caller"), caller.to_owned()), (String::from("host"), host.to_owned())];
	if let Some(outcome) = outcome {
		labels.push((String::from("outcome"), outcome.to_owned()));
	}
	labels.sort();
	labels
}

#[tokio::test]
async fn a_request_is_counted_by_caller_host_and_outcome_and_nothing_else() {
	let recorder = DebuggingRecorder::new();
	let _guard = metrics::set_default_local_recorder(&recorder);
	let ok = answering("200 OK", "hello").await.unwrap();
	let refused = answering("503 Service Unavailable", "").await.unwrap();
	let client = outbound::client(Caller::GameFeed).no_proxy().timeout(Duration::from_secs(5)).build().unwrap();

	client.post(ok.as_str()).body("abc").send().await.unwrap();
	client.get(refused.as_str()).send().await.unwrap();
	// Nothing listens on port 9 of loopback.
	client.get("http://127.0.0.1:9/").send().await.unwrap_err();

	let seen = counters(&recorder);
	let want = vec![
		(String::from(RECEIVED_METRIC), labels("game_feed", "espn", None), 5),
		(String::from(REQUESTS_METRIC), labels("game_feed", "espn", Some("no_answer")), 1),
		(String::from(REQUESTS_METRIC), labels("game_feed", "espn", Some("ok")), 1),
		(String::from(REQUESTS_METRIC), labels("game_feed", "espn", Some("refused_5xx")), 1),
		(String::from(SENT_METRIC), labels("game_feed", "espn", None), 3),
	];
	assert_eq!(seen, want);
}

#[tokio::test]
async fn a_request_that_does_not_build_is_not_recorded() {
	let recorder = DebuggingRecorder::new();
	let _guard = metrics::set_default_local_recorder(&recorder);
	let client = outbound::client(Caller::Mailbox).build().unwrap();
	client.get("not a url").send().await.unwrap_err();
	assert!(counters(&recorder).is_empty());
}
