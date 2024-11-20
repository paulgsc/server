use chromiumoxide::Browser;
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio;
use tokio::sync::Mutex;
use tokio::time;

#[derive(Debug, Clone)]
struct TabStats {
	domain: String,
	url: String,
	start_time: Instant,
	total_time: Duration,
}

#[derive(Debug)]
struct DomainStats {
	total_time: Duration,
	urls: Vec<String>,
}

fn extract_domain(url: &str) -> String {
	// Simple domain extraction
	url
		.split("://")
		.nth(1) // Remove protocol
		.map(|s| s.split('/').next().unwrap_or("unknown"))
		.map(|s| s.split(':').next().unwrap_or("unknown")) // Remove port if exists
		.map(|s| s.to_string())
		.unwrap_or_else(|| "unknown".to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	// Connect to existing Chrome instance
	let websocket_url = "ws://localhost:9222/devtools/browser/04024350-7cfa-4553-b3dc-fb7a68012514";
	let (browser, mut handler) = Browser::connect(websocket_url).await.expect("Failed to connect to Chrome");

	// Spawn browser handler
	tokio::spawn(async move {
		while let Some(h) = handler.next().await {
			if let Err(e) = h {
				eprintln!("Error: {:?}", e);
			}
		}
	});

	let tab_stats = Arc::new(Mutex::new(HashMap::new()));
	let domain_stats = Arc::new(Mutex::new(HashMap::new()));

	// Initialize with current tabs
	let pages = browser.pages().await?;
	for page in pages {
		if let Ok(Some(url)) = page.url().await {
			let url = url.to_string(); // Convert to owned String
			let domain = extract_domain(&url);
			let tab_id = format!("{:?}", page.target_id()); // Use debug representation

			let mut stats = tab_stats.lock().await;
			stats.insert(
				tab_id,
				TabStats {
					domain: domain.clone(),
					url,
					start_time: Instant::now(),
					total_time: Duration::from_secs(0),
				},
			);
		}
	}

	// Clone Arc for the periodic update task
	let tab_stats_clone = Arc::clone(&tab_stats);
	let domain_stats_clone = Arc::clone(&domain_stats);

	// Spawn periodic update task
	tokio::spawn(async move {
		let mut interval = time::interval(Duration::from_secs(5));
		loop {
			interval.tick().await;
			update_and_print_stats(Arc::clone(&tab_stats_clone), Arc::clone(&domain_stats_clone)).await;
		}
	});

	// Since pages_stream is not available, we'll use a different approach
	// This might require modifications based on the specific chromiumoxide version
	loop {
		let current_pages = browser.pages().await?;

		for page in current_pages {
			if let Ok(Some(url)) = page.url().await {
				let url = url.to_string(); // Convert to owned String
				let domain = extract_domain(&url);
				let tab_id = format!("{:?}", page.target_id()); // Use debug representation

				let mut stats = tab_stats.lock().await;

				// Update or create stats for this tab
				if let Some(tab_stat) = stats.get_mut(&tab_id) {
					tab_stat.total_time += tab_stat.start_time.elapsed();
					tab_stat.start_time = Instant::now();
					tab_stat.url = url;
					tab_stat.domain = domain;
				} else {
					stats.insert(
						tab_id,
						TabStats {
							domain,
							url,
							start_time: Instant::now(),
							total_time: Duration::from_secs(0),
						},
					);
				}
			}
		}

		// Wait a bit before checking pages again
		tokio::time::sleep(Duration::from_secs(1)).await;
	}
}

async fn update_and_print_stats(tab_stats: Arc<Mutex<HashMap<String, TabStats>>>, domain_stats: Arc<Mutex<HashMap<String, DomainStats>>>) {
	let stats = tab_stats.lock().await;
	let mut domain_stats = domain_stats.lock().await;

	// Update domain statistics
	domain_stats.clear();
	for (_, tab_stat) in stats.iter() {
		let total_time = tab_stat.total_time + tab_stat.start_time.elapsed();

		domain_stats
			.entry(tab_stat.domain.clone())
			.and_modify(|d| {
				d.total_time += total_time;
				if !d.urls.contains(&tab_stat.url) {
					d.urls.push(tab_stat.url.clone());
				}
			})
			.or_insert(DomainStats {
				total_time,
				urls: vec![tab_stat.url.clone()],
			});
	}

	// Print statistics
	println!("\n=== Tab Statistics ===");
	println!("Total tabs: {}", stats.len());

	println!("\n=== Domain Statistics ===");
	for (domain, stats) in domain_stats.iter() {
		println!(
			"\nDomain: {} \nTotal time: {:.2}m \nUnique URLs: {}",
			domain,
			stats.total_time.as_secs_f64() / 60.0,
			stats.urls.len()
		);
		for url in &stats.urls {
			println!("  - {}", url);
		}
	}
}
