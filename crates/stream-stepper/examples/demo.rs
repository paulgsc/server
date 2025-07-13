// examples/livestream_timeline.rs
use stream_stepper::*;
use uuid::Uuid;

fn main() -> Result<()> {
	let mut chapters = LiveChapters::new();

	// Simulate livestream events at different times

	// Time 0ms: Start stream with intro
	let events_t0 = vec![TimelineEvent::StartChapter {
		uid: Uuid::new_v4().to_string(), // Use UUID for unique IDs
		context: Context::new("Stream Introduction").with_tag("phase", "opening"),
		start_time: 0,
		payload: Payload::new(serde_json::json!({
			"description": "Welcome to the coding stream!",
			"viewer_count": 10
		}))?,
	}];

	let snapshot_t0 = chapters.process_events_at_time(events_t0, 0)?;
	println!("--- Time: 0ms ---");
	print_snapshot(&snapshot_t0);

	// Time 30,000ms: End intro, start coding
	let intro_uid = snapshot_t0.segments[0].chapters[0].uid.clone(); // Get the UID of the intro chapter
	let coding_uid = Uuid::new_v4().to_string();

	let events_t30 = vec![
		TimelineEvent::EndChapter {
			uid: intro_uid,
			end_time: 30_000,
			final_payload: Some(Payload::new(serde_json::json!({"conclusion": "Intro finished"}))?),
		},
		TimelineEvent::StartChapter {
			uid: coding_uid.clone(),
			context: Context::new("Coding Session: Rust & WebAssembly").with_tag("topic", "rust").with_tag("tech", "wasm"),
			start_time: 30_000,
			payload: Payload::new(serde_json::json!({
				"language": "Rust",
				"project": "Timeline UI"
			}))?,
		},
	];

	let snapshot_t30 = chapters.process_events_at_time(events_t30, 30_000)?;
	println!("\n--- Time: 30,000ms ---");
	print_snapshot(&snapshot_t30);

	// Time 60,000ms: Update coding chapter payload (e.g., viewer count changes)
	let events_t60 = vec![TimelineEvent::UpdatePayload {
		uid: coding_uid.clone(),
		payload: Payload::new(serde_json::json!({
			"language": "Rust",
			"project": "Timeline UI",
			"viewer_count": 50,
			"progress": "Implementing generate_timeline_snapshot"
		}))?,
	}];

	let snapshot_t60 = chapters.process_events_at_time(events_t60, 60_000)?;
	println!("\n--- Time: 60,000ms ---");
	print_snapshot(&snapshot_t60);

	// Time 90,000ms: Start Q&A session, coding continues in background (overlapping chapters)
	let qa_uid = Uuid::new_v4().to_string();
	let events_t90 = vec![TimelineEvent::StartChapter {
		uid: qa_uid.clone(),
		context: Context::new("Q&A Session").with_tag("type", "interactive"),
		start_time: 90_000,
		payload: Payload::new(serde_json::json!({
			"questions_asked": 3
		}))?,
	}];

	let snapshot_t90 = chapters.process_events_at_time(events_t90, 90_000)?;
	println!("\n--- Time: 90,000ms ---");
	print_snapshot(&snapshot_t90);

	// Time 120,000ms: End Q&A, continue coding. Introduce a short "break" chapter.
	let break_uid = Uuid::new_v4().to_string();
	let events_t120 = vec![
		TimelineEvent::EndChapter {
			uid: qa_uid,
			end_time: 120_000,
			final_payload: Some(Payload::new(serde_json::json!({"questions_answered": 10}))?),
		},
		TimelineEvent::StartChapter {
			uid: break_uid.clone(),
			context: Context::new("Short Break").with_tag("type", "intermission"),
			start_time: 120_000,
			payload: Payload::empty(),
		},
	];

	let snapshot_t120 = chapters.process_events_at_time(events_t120, 120_000)?;
	println!("\n--- Time: 120,000ms ---");
	print_snapshot(&snapshot_t120);

	// Time 130,000ms: End break, back to coding. Also, update coding chapter's title slightly.
	let events_t130 = vec![
		TimelineEvent::EndChapter {
			uid: break_uid,
			end_time: 130_000,
			final_payload: None,
		},
		TimelineEvent::UpdateContext {
			uid: coding_uid.clone(),
			context: Context::new("Deep Dive: Rust Async").with_tag("topic", "rust").with_tag("tech", "async"),
		},
	];

	let snapshot_t130 = chapters.process_events_at_time(events_t130, 130_000)?;
	println!("\n--- Time: 130,000ms ---");
	print_snapshot(&snapshot_t130);

	// Time 180,000ms: Livestream ends. Complete the active coding chapter.
	let events_t180 = vec![TimelineEvent::CompleteChapter {
		uid: coding_uid,
		completion_time: 180_000,
		final_payload: Payload::new(serde_json::json!({
			"status": "Completed",
			"final_viewer_count": 75,
			"summary": "Finished implementing timeline generation logic."
		}))?,
	}];

	let snapshot_t180 = chapters.process_events_at_time(events_t180, 180_000)?;
	println!("\n--- Time: 180,000ms (Stream End) ---");
	print_snapshot(&snapshot_t180);

	// Time 200,000ms: Get snapshot without new events (time advances)
	let snapshot_t200 = chapters.get_timeline_snapshot(200_000)?;
	println!("\n--- Time: 200,000ms (Post-stream) ---");
	print_snapshot(&snapshot_t200);

	// Clear all chapters
	let events_clear = vec![TimelineEvent::ClearAll];
	let snapshot_clear = chapters.process_events_at_time(events_clear, 200_000)?;
	println!("\n--- After Clear All ---");
	print_snapshot(&snapshot_clear);

	Ok(())
}

fn print_snapshot(snapshot: &TimelineSnapshot) {
	println!("  Current Time: {}ms", snapshot.current_time);
	println!("  Total Duration: {}ms", snapshot.total_duration);
	println!("  Active Chapters Count: {}", snapshot.active_count);
	println!("  State Version: {}", snapshot.version);
	println!("  Segments:");
	for (i, segment) in snapshot.segments.iter().enumerate() {
		let end_time_display = segment.end_time.map_or("ACTIVE".to_string(), |t| format!("{}ms", t));
		println!(
			"    Segment {}: [{}ms - {}] ({}ms, {:.2}%) - '{}' ({})",
			i,
			segment.start_time,
			end_time_display,
			segment.duration,
			segment.percentage,
			segment.title,
			if segment.is_active { "Active" } else { "Inactive" }
		);
		for chapter in &segment.chapters {
			let chapter_end_time = chapter.time_range.end.map_or("None".to_string(), |t| t.to_string());
			println!(
				"      - Chapter: '{}' (UID: {}, [{} - {}], Created: {})",
				chapter.context.title, chapter.uid, chapter.time_range.start, chapter_end_time, chapter.created_at
			);
		}
	}
}
