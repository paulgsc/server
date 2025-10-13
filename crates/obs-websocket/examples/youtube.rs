#![allow(dead_code)]

use obs_websocket::*;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

	// Run the YouTube streaming showcase
	showcase_youtube_streaming().await?;
	Ok(())
}

pub async fn showcase_youtube_streaming() -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("📺 Starting YouTube Streaming Showcase");

	let obs_config = ObsConfig::default();
	let obs_manager = ObsWebSocketManager::new(obs_config, RetryConfig::default());

	// Create cancellation token for graceful shutdown
	let cancel_token = CancellationToken::new();
	let cancel_clone = cancel_token.clone();

	// Setup shutdown handler
	tokio::spawn(async move {
		let _ = tokio::signal::ctrl_c().await;
		tracing::info!("🛑 Shutdown signal received");
		cancel_clone.cancel();
	});

	// Connect to OBS with basic polling configuration
	let polling_requests = PollingConfig::default();

	match obs_manager.connect(polling_requests).await {
		Ok(()) => {
			tracing::info!("✅ Connected to OBS WebSocket successfully");

			// Show connection info
			if let Ok(info) = obs_manager.connection_info().await {
				tracing::info!("📊 Connection Info: State={:?}", info.state);
			}

			// Run YouTube streaming demonstrations
			demonstrate_youtube_streaming(&obs_manager, &cancel_token).await?;
		}
		Err(e) => {
			tracing::error!("❌ Failed to connect to OBS: {}", e);
			return Err(e.into());
		}
	}

	// Clean disconnect
	let _ = obs_manager.disconnect().await;
	tracing::info!("🏁 YouTube Streaming Showcase completed");

	Ok(())
}

async fn demonstrate_youtube_streaming(obs_manager: &ObsWebSocketManager, cancel_token: &CancellationToken) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("📺 Starting YouTube streaming demonstrations...");

	let demonstrations = [
		"🎮 Gaming Stream Setup",
		"📚 Educational Content Setup",
		"🧪 Private Test Stream",
		"🎵 Music & Entertainment",
		"⏰ Dynamic Time-based Config",
		"🔄 Mid-stream Updates",
		"🏷️ Advanced Tagging Strategies",
		"🎯 Niche Content Examples",
	];

	for (i, category) in demonstrations.iter().enumerate() {
		if cancel_token.is_cancelled() {
			tracing::info!("⏹️ Demonstration cancelled by user");
			break;
		}

		tracing::info!("🔄 Running: {}", category);

		let result = match i {
			0 => demo_gaming_stream(obs_manager).await,
			// 1 => demo_educational_stream(obs_manager).await,
			// 2 => demo_private_test_stream(obs_manager).await,
			// 3 => demo_music_entertainment(obs_manager).await,
			// 4 => demo_dynamic_config(obs_manager).await,
			// 5 => demo_midstream_updates(obs_manager).await,
			// 6 => demo_advanced_tagging(obs_manager).await,
			// 7 => demo_niche_content(obs_manager).await,
			_ => Ok(()),
		};

		match result {
			Ok(()) => tracing::info!("✅ {} completed successfully", category),
			Err(e) => tracing::warn!("⚠️ {} encountered error: {}", category, e),
		}

		// Pause between demonstrations
		if !cancel_token.is_cancelled() {
			sleep(Duration::from_millis(1000)).await;
		}
	}

	Ok(())
}

// Gaming stream configurations
async fn demo_gaming_stream(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  🎮 Setting up gaming stream configurations...");

	// Popular game streaming setup
	let gaming_config = ObsCommand::SetYouTubeStream {
		stream_key: "j8z8-cqdk-p09j-tbtb-cxc1".to_string(),
		title: "🔴 LIVE: Elden Ring Blind Playthrough | First Time Playing!".to_string(),
		description: concat!(
			"Welcome to my first playthrough of Elden Ring! Join me as I explore this incredible world blind. ",
			"No spoilers please! 🛡️⚔️\n\n",
			"⏰ Stream Schedule: Mon/Wed/Fri 7PM EST\n",
			"💬 Chat Rules: Be kind, no spoilers, have fun!\n",
			"🔔 Don't forget to subscribe and hit the notification bell!"
		)
		.to_string(),
		category: "Gaming".to_string(),
		privacy: YouTubePrivacy::Public,
		unlisted: false,
		tags: vec![
			"elden ring".to_string(),
			"gaming".to_string(),
			"live".to_string(),
			"blind playthrough".to_string(),
			"souls game".to_string(),
			"first time".to_string(),
			"no spoilers".to_string(),
		],
	};

	match obs_manager.execute_command(gaming_config).await {
		Ok(()) => tracing::info!("  ✅ Gaming stream configured: Elden Ring"),
		Err(e) => tracing::debug!("  ℹ️ Gaming config: {} (requires stream key)", e),
	}

	sleep(Duration::from_millis(100)).await;

	tracing::info!("🚀 Starting stream...");
	match obs_manager.execute_command(ObsCommand::StartStream).await {
		Ok(()) => tracing::info!("  ✅ Start stream command executed"),
		Err(e) => tracing::debug!("  ℹ️ Start stream: {} (expected if not configured)", e),
	}

	Ok(())
}

// Educational content streaming
async fn demo_educational_stream(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  📚 Setting up educational content streams...");

	// Programming tutorial
	let programming_config = ObsCommand::SetYouTubeStream {
		stream_key: "edu-programming-key".to_string(),
		title: "🦀 Rust Programming: Building a WebSocket Server from Scratch".to_string(),
		description: concat!(
			"Learn Rust by building a real-time WebSocket server! This tutorial covers:\n\n",
			"📋 What we'll learn:\n",
			"• Async programming in Rust\n",
			"• WebSocket protocol fundamentals\n",
			"• Error handling patterns\n",
			"• Testing async code\n",
			"• Performance optimization\n\n",
			"💻 Prerequisites: Basic Rust knowledge\n",
			"📂 Code: github.com/example/websocket-tutorial\n",
			"⏰ Duration: ~2 hours"
		)
		.to_string(),
		category: "Education".to_string(),
		privacy: YouTubePrivacy::Unlisted, // Unlisted for course material
		unlisted: true,
		tags: vec![
			"rust programming".to_string(),
			"websocket".to_string(),
			"tutorial".to_string(),
			"coding".to_string(),
			"async programming".to_string(),
			"software development".to_string(),
			"programming tutorial".to_string(),
		],
	};

	match obs_manager.execute_command(programming_config).await {
		Ok(()) => tracing::info!("  ✅ Programming tutorial configured"),
		Err(e) => tracing::debug!("  ℹ️ Programming config: {} (requires stream key)", e),
	}

	sleep(Duration::from_millis(200)).await;

	// Science/Math content
	let science_config = ObsCommand::SetYouTubeStream {
		stream_key: "science-stream-key".to_string(),
		title: "🧬 Interactive Biology: Exploring Cellular Processes in 3D".to_string(),
		description: concat!(
			"Journey into the microscopic world! Using interactive 3D models to understand:\n\n",
			"🔬 Today's topics:\n",
			"• Mitochondrial structure and function\n",
			"• Protein synthesis visualization\n",
			"• Cell membrane dynamics\n",
			"• Q&A session\n\n",
			"Perfect for high school and college students! Ask questions in chat 💬"
		)
		.to_string(),
		category: "Education".to_string(),
		privacy: YouTubePrivacy::Public,
		unlisted: false,
		tags: vec![
			"biology".to_string(),
			"science education".to_string(),
			"3d visualization".to_string(),
			"cellular biology".to_string(),
			"interactive learning".to_string(),
			"student friendly".to_string(),
		],
	};

	match obs_manager.execute_command(science_config).await {
		Ok(()) => tracing::info!("  ✅ Science education stream configured"),
		Err(e) => tracing::debug!("  ℹ️ Science config: {} (requires stream key)", e),
	}

	Ok(())
}

// Private test stream setup
async fn demo_private_test_stream(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  🧪 Setting up private test streams...");

	// Technical test stream
	let test_config = ObsCommand::SetYouTubeStream {
		stream_key: "private-test-key".to_string(),
		title: "🔧 Stream Test - Audio/Video Quality Check".to_string(),
		description: concat!(
			"PRIVATE TEST STREAM - Please ignore\n\n",
			"Testing:\n",
			"- Video bitrate and quality\n",
			"- Audio levels and clarity\n",
			"- Scene transitions\n",
			"- OBS plugin compatibility\n",
			"- Network stability"
		)
		.to_string(),
		category: "Science & Technology".to_string(),
		privacy: YouTubePrivacy::Private,
		unlisted: false,
		tags: vec!["test".to_string(), "technical".to_string(), "quality check".to_string()],
	};

	match obs_manager.execute_command(test_config).await {
		Ok(()) => tracing::info!("  ✅ Private test stream configured"),
		Err(e) => tracing::debug!("  ℹ️ Test config: {} (requires stream key)", e),
	}

	sleep(Duration::from_millis(200)).await;

	// Rehearsal stream
	let rehearsal_config = ObsCommand::SetYouTubeStream {
		stream_key: "rehearsal-key".to_string(),
		title: "🎭 Presentation Rehearsal - Internal Use".to_string(),
		description: "Practice run for upcoming public presentation. Testing timing and content flow.".to_string(),
		category: "Education".to_string(),
		privacy: YouTubePrivacy::Private,
		unlisted: false,
		tags: vec!["rehearsal".to_string(), "practice".to_string(), "internal".to_string()],
	};

	match obs_manager.execute_command(rehearsal_config).await {
		Ok(()) => tracing::info!("  ✅ Rehearsal stream configured"),
		Err(e) => tracing::debug!("  ℹ️ Rehearsal config: {} (requires stream key)", e),
	}

	Ok(())
}

// Music and entertainment streaming
async fn demo_music_entertainment(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  🎵 Setting up music and entertainment streams...");

	// Lo-fi music stream
	let lofi_config = ObsCommand::SetYouTubeStream {
		stream_key: "lofi-music-key".to_string(),
		title: "🎧 24/7 Lo-Fi Hip Hop - Study, Relax, Chill 🌙".to_string(),
		description: concat!(
			"Continuous lo-fi beats for studying, working, and relaxing. ",
			"Perfect background music for productivity! 📚✨\n\n",
			"🎵 Features:\n",
			"• Hand-picked lo-fi tracks\n",
			"• No interruptions\n",
			"• Perfect for focus\n",
			"• Rainy day vibes\n\n",
			"💙 Support the artists - links in description"
		)
		.to_string(),
		category: "Music".to_string(),
		privacy: YouTubePrivacy::Public,
		unlisted: false,
		tags: vec![
			"lo-fi".to_string(),
			"hip hop".to_string(),
			"study music".to_string(),
			"chill beats".to_string(),
			"relaxing music".to_string(),
			"24/7 stream".to_string(),
			"productivity".to_string(),
			"focus music".to_string(),
		],
	};

	match obs_manager.execute_command(lofi_config).await {
		Ok(()) => tracing::info!("  ✅ Lo-fi music stream configured"),
		Err(e) => tracing::debug!("  ℹ️ Lo-fi config: {} (requires stream key)", e),
	}

	sleep(Duration::from_millis(200)).await;

	// Interactive music session
	let interactive_music_config = ObsCommand::SetYouTubeStream {
		stream_key: "interactive-music-key".to_string(),
		title: "🎸 Live Jam Session - Taking Song Requests! 🎶".to_string(),
		description: concat!(
			"Interactive music session with live requests! Bring your favorite songs and let's play together! 🎵\n\n",
			"🎯 How it works:\n",
			"• Drop song requests in chat\n",
			"• I'll play them live (if I know them!)\n",
			"• Learn new songs together\n",
			"• Acoustic covers and originals\n\n",
			"🎸 Instruments: Guitar, Piano, Voice\n",
			"💝 Tips appreciated but never required!"
		)
		.to_string(),
		category: "Music".to_string(),
		privacy: YouTubePrivacy::Public,
		unlisted: false,
		tags: vec![
			"live music".to_string(),
			"acoustic".to_string(),
			"song requests".to_string(),
			"interactive".to_string(),
			"guitar".to_string(),
			"covers".to_string(),
			"jam session".to_string(),
		],
	};

	match obs_manager.execute_command(interactive_music_config).await {
		Ok(()) => tracing::info!("  ✅ Interactive music session configured"),
		Err(e) => tracing::debug!("  ℹ️ Interactive music config: {} (requires stream key)", e),
	}

	Ok(())
}

// Dynamic time-based configuration
async fn demo_dynamic_config(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  ⏰ Demonstrating dynamic time-based configurations...");

	// Get current hour for dynamic configuration
	let current_hour = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() / 3600 % 24;

	let (title_prefix, category, description, tags) = match current_hour {
		6..=11 => (
			"🌅 Good Morning Stream",
			"People & Blogs",
			"Starting the day with positive energy! Morning coffee, planning, and chill conversation. What are your goals for today? ☕",
			vec!["morning".to_string(), "coffee".to_string(), "motivation".to_string(), "daily routine".to_string()],
		),
		12..=17 => (
			"☀️ Afternoon Productivity",
			"Education",
			"Afternoon work session! Join me for some productive coding, learning, and getting things done! 💼",
			vec!["productivity".to_string(), "work".to_string(), "coding".to_string(), "afternoon".to_string()],
		),
		18..=22 => (
			"🌆 Evening Hangout",
			"Entertainment",
			"Winding down with some evening entertainment! Games, music, and good vibes. Come relax with us! 🎮",
			vec!["evening".to_string(), "chill".to_string(), "entertainment".to_string(), "hangout".to_string()],
		),
		_ => (
			"🌙 Late Night Vibes",
			"Entertainment",
			"Late night creative session! Perfect time for deep work, creative projects, and quiet conversation. 🌃",
			vec!["late night".to_string(), "creative".to_string(), "quiet".to_string(), "focus".to_string()],
		),
	};

	let dynamic_config = ObsCommand::SetYouTubeStream {
		stream_key: "dynamic-time-key".to_string(),
		title: format!("{} - Live Now!", title_prefix),
		description: description.to_string(),
		category: category.to_string(),
		privacy: YouTubePrivacy::Public,
		unlisted: false,
		tags: {
			let mut all_tags = tags;
			all_tags.extend(vec!["live".to_string(), "interactive".to_string()]);
			all_tags
		},
	};

	match obs_manager.execute_command(dynamic_config).await {
		Ok(()) => tracing::info!("  ✅ Dynamic config set for hour {}: {}", current_hour, title_prefix),
		Err(e) => tracing::debug!("  ℹ️ Dynamic config: {} (requires stream key)", e),
	}

	Ok(())
}

// Mid-stream updates demonstration
async fn demo_midstream_updates(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  🔄 Demonstrating mid-stream updates...");

	// Initial stream setup
	let initial_config = ObsCommand::SetYouTubeStream {
		stream_key: "update-demo-key".to_string(),
		title: "🔴 Live Stream - Just Getting Started!".to_string(),
		description: "Starting our stream! More details coming soon...".to_string(),
		category: "Entertainment".to_string(),
		privacy: YouTubePrivacy::Public,
		unlisted: false,
		tags: vec!["live".to_string(), "starting".to_string()],
	};

	match obs_manager.execute_command(initial_config).await {
		Ok(()) => tracing::info!("  ✅ Initial stream configuration set"),
		Err(e) => tracing::debug!("  ℹ️ Initial config: {} (requires stream key)", e),
	}

	sleep(Duration::from_millis(300)).await;

	// Update 1: Add topic
	let topic_update = ObsCommand::SetYouTubeStream {
		stream_key: "update-demo-key".to_string(),
		title: "🔴 Live: Today's Topic - Building Rust Applications!".to_string(),
		description: concat!(
			"Now live! Today we're diving into Rust application development.\n\n",
			"📋 Agenda:\n",
			"• Project setup and structure\n",
			"• Working with async code\n",
			"• Error handling patterns\n",
			"• Q&A session\n\n",
			"Join the conversation in chat! 💬"
		)
		.to_string(),
		category: "Education".to_string(),
		privacy: YouTubePrivacy::Public,
		unlisted: false,
		tags: vec!["rust".to_string(), "programming".to_string(), "live coding".to_string(), "tutorial".to_string()],
	};

	match obs_manager.execute_command(topic_update).await {
		Ok(()) => tracing::info!("  ✅ Stream updated with topic information"),
		Err(e) => tracing::debug!("  ℹ️ Topic update: {} (requires stream key)", e),
	}

	sleep(Duration::from_millis(300)).await;

	// Update 2: Taking questions
	let questions_update = ObsCommand::SetYouTubeStream {
		stream_key: "update-demo-key".to_string(),
		title: "🔴 Live: Rust Development + Q&A - Ask Questions!".to_string(),
		description: concat!(
			"We're now in the Q&A portion! Ask your Rust questions in chat! 🙋‍♀️🙋‍♂️\n\n",
			"✅ Covered so far:\n",
			"• Project setup ✓\n",
			"• Async fundamentals ✓\n",
			"• Error handling ✓\n",
			"• NOW: Your questions!\n\n",
			"Don't be shy - all skill levels welcome! 🎓"
		)
		.to_string(),
		category: "Education".to_string(),
		privacy: YouTubePrivacy::Public,
		unlisted: false,
		tags: vec![
			"rust".to_string(),
			"programming".to_string(),
			"q&a".to_string(),
			"questions".to_string(),
			"interactive".to_string(),
		],
	};

	match obs_manager.execute_command(questions_update).await {
		Ok(()) => tracing::info!("  ✅ Stream updated for Q&A session"),
		Err(e) => tracing::debug!("  ℹ️ Q&A update: {} (requires stream key)", e),
	}

	Ok(())
}

// Advanced tagging strategies
async fn demo_advanced_tagging(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  🏷️ Demonstrating advanced tagging strategies...");

	// SEO-optimized tags for discoverability
	let seo_optimized_config = ObsCommand::SetYouTubeStream {
		stream_key: "seo-optimized-key".to_string(),
		title: "Complete Beginner's Guide to Web Development 2024".to_string(),
		description: concat!(
			"Learn web development from scratch! Perfect for beginners starting their coding journey in 2024.\n\n",
			"🎯 What you'll learn:\n",
			"• HTML5 fundamentals\n",
			"• CSS Grid and Flexbox\n",
			"• JavaScript ES2024 features\n",
			"• Modern development tools\n",
			"• Building your first website\n\n",
			"🔗 Free resources: github.com/webdev-2024\n",
			"💼 Career tips included!"
		)
		.to_string(),
		category: "Education".to_string(),
		privacy: YouTubePrivacy::Public,
		unlisted: false,
		tags: vec![
			// Primary keywords
			"web development".to_string(),
			"beginners guide".to_string(),
			"learn to code".to_string(),
			// Technology-specific
			"html5".to_string(),
			"css3".to_string(),
			"javascript".to_string(),
			// Intent-based
			"web development tutorial".to_string(),
			"coding for beginners".to_string(),
			"how to learn programming".to_string(),
			// Year-specific for relevance
			"web development 2024".to_string(),
			"modern web development".to_string(),
			// Audience-specific
			"beginner friendly".to_string(),
			"step by step tutorial".to_string(),
		],
	};

	match obs_manager.execute_command(seo_optimized_config).await {
		Ok(()) => tracing::info!("  ✅ SEO-optimized tagging configured"),
		Err(e) => tracing::debug!("  ℹ️ SEO config: {} (requires stream key)", e),
	}

	sleep(Duration::from_millis(200)).await;

	// Long-tail keyword strategy
	let longtail_config = ObsCommand::SetYouTubeStream {
		stream_key: "longtail-seo-key".to_string(),
		title: "How to Build a REST API with Rust and Actix-Web for Beginners".to_string(),
		description: "Specific, detailed tutorial on building REST APIs using Rust and the Actix-Web framework.".to_string(),
		category: "Education".to_string(),
		privacy: YouTubePrivacy::Public,
		unlisted: false,
		tags: vec![
			// Very specific long-tail keywords
			"rust rest api tutorial".to_string(),
			"actix web framework".to_string(),
			"rust web development".to_string(),
			"how to build api rust".to_string(),
			"rest api from scratch".to_string(),
			"rust backend development".to_string(),
			"actix web tutorial".to_string(),
			"rust programming tutorial".to_string(),
		],
	};

	match obs_manager.execute_command(longtail_config).await {
		Ok(()) => tracing::info!("  ✅ Long-tail keyword strategy configured"),
		Err(e) => tracing::debug!("  ℹ️ Long-tail config: {} (requires stream key)", e),
	}

	Ok(())
}

// Niche content examples
async fn demo_niche_content(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  🎯 Demonstrating niche content configurations...");

	// Specific hobby/interest stream
	let hobby_config = ObsCommand::SetYouTubeStream {
		stream_key: "mechanical-keyboards-key".to_string(),
		title: "🔤 Custom Mechanical Keyboard Build - Lubing Switches Live!".to_string(),
		description: concat!(
			"Building a custom 75% keyboard from scratch! Today we're lubing Gateron Oil King switches. ",
			"Perfect for keyboard enthusiasts! 🔧⌨️\n\n",
			"🛠️ Build specs:\n",
			"• Akko Mod007B PCB\n",
			"• Gateron Oil King switches\n",
			"• Durock V2 stabilizers\n",
			"• GMK keycaps\n\n",
			"💡 Tips for beginners included!\n",
			"🛒 Parts list in description"
		)
		.to_string(),
		category: "Science & Technology".to_string(),
		privacy: YouTubePrivacy::Public,
		unlisted: false,
		tags: vec![
			"mechanical keyboards".to_string(),
			"custom keyboard".to_string(),
			"keyboard build".to_string(),
			"switch lubing".to_string(),
			"gateron oil king".to_string(),
			"keyboard mods".to_string(),
			"enthusiast".to_string(),
			"diy keyboard".to_string(),
		],
	};

	match obs_manager.execute_command(hobby_config).await {
		Ok(()) => tracing::info!("  ✅ Niche hobby stream configured"),
		Err(e) => tracing::debug!("  ℹ️ Hobby config: {} (requires stream key)", e),
	}

	sleep(Duration::from_millis(200)).await;

	// Professional/career-focused content
	let professional_config = ObsCommand::SetYouTubeStream {
		stream_key: "career-advice-key".to_string(),
		title: "💼 Tech Career AMA: Senior Developer Career Path Q&A".to_string(),
		description: concat!(
			"Ask a Senior Software Engineer anything about tech careers! 15+ years in the industry. ",
			"Let's talk about growth, interviews, salary negotiation, and more! 🚀\n\n",
			"📈 Topics we can cover:\n",
			"• Junior to Senior progression\n",
			"• Technical interview prep\n",
			"• Salary negotiation strategies\n",
			"• Remote work tips\n",
			"• Career switching advice\n",
			"• Leadership transition\n\n",
			"Ask your questions in chat! All experience levels welcome 🎯"
		)
		.to_string(),
		category: "Education".to_string(),
		privacy: YouTubePrivacy::Public,
		unlisted: false,
		tags: vec![
			"tech career".to_string(),
			"software engineer".to_string(),
			"career advice".to_string(),
			"programming career".to_string(),
			"senior developer".to_string(),
			"tech industry".to_string(),
			"career growth".to_string(),
			"interview tips".to_string(),
			"salary negotiation".to_string(),
		],
	};

	match obs_manager.execute_command(professional_config).await {
		Ok(()) => tracing::info!("  ✅ Professional content stream configured"),
		Err(e) => tracing::debug!("  ℹ️ Professional config: {} (requires stream key)", e),
	}

	Ok(())
}
