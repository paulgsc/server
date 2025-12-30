use inquire::{Confirm, CustomType, Text};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Region {
	Video,
	Title,
	MainContent,
	FooterLeft,
	SidebarTop,
	SidebarBottom,
	FooterRight,
}

impl std::fmt::Display for Region {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.as_str())
	}
}

impl Region {
	fn as_str(&self) -> &str {
		match self {
			Region::Video => "video",
			Region::Title => "title",
			Region::MainContent => "mainContent",
			Region::FooterLeft => "footerLeft",
			Region::SidebarTop => "sidebarTop",
			Region::SidebarBottom => "sidebarBottom",
			Region::FooterRight => "footerRight",
		}
	}

	fn all() -> Vec<Region> {
		vec![
			Region::Video,
			Region::Title,
			Region::MainContent,
			Region::FooterLeft,
			Region::SidebarTop,
			Region::SidebarBottom,
			Region::FooterRight,
		]
	}
}

#[derive(Debug, Serialize)]
struct ComponentPlacement {
	#[serde(rename = "registryKey")]
	registry_key: String,
	duration: u64,
	props: Value,
}

#[derive(Debug, Serialize)]
struct CubeProps {
	region: String,
	#[serde(rename = "faceCapacity")]
	face_capacity: u32,
	#[serde(skip_serializing_if = "Option::is_none")]
	children: Option<Vec<ComponentPlacement>>,
}

#[derive(Debug, Serialize)]
struct PanelIntent {
	#[serde(rename = "registryKey")]
	registry_key: String,
	props: Value,
	#[serde(skip_serializing_if = "Option::is_none")]
	focus: Option<Value>,
}

#[derive(Debug, Serialize)]
struct UILayoutIntent {
	panels: HashMap<String, PanelIntent>,
}

#[derive(Debug, Serialize)]
struct SceneConfig {
	scene_name: String,
	duration: u64,
	start_time: u64,
	ui: Vec<UILayoutIntent>,
}

#[derive(Debug, Serialize)]
struct ConfigOutput {
	#[serde(flatten)]
	scenes: HashMap<String, SceneConfig>,
}

struct SceneDefinition {
	name: String,
	duration: u64,
	panels: Vec<PanelDefinition>,
}

struct PanelDefinition {
	region: Region,
	children_count: usize,
	has_focus: bool,
	focus_intensity: f64,
}

fn generate_child_placements(count: usize, parent_duration: u64) -> Vec<ComponentPlacement> {
	(0..count)
		.map(|_| ComponentPlacement {
			registry_key: "TODO_REGISTRY_KEY".to_string(),
			duration: parent_duration,
			props: json!({
					"className": "size-full",
					// Add more template props here
			}),
		})
		.collect()
}

fn build_panel(panel_def: &PanelDefinition, scene_duration: u64) -> (String, PanelIntent) {
	let region_str = panel_def.region.as_str();

	let children = if panel_def.children_count > 0 {
		Some(generate_child_placements(panel_def.children_count, scene_duration))
	} else {
		None
	};

	let cube_props = CubeProps {
		region: region_str.to_string(),
		face_capacity: 1,
		children,
	};

	let focus = if panel_def.has_focus {
		Some(json!({
				"region": region_str,
				"intensity": panel_def.focus_intensity
		}))
	} else {
		None
	};

	let panel = PanelIntent {
		registry_key: "cube".to_string(),
		props: serde_json::to_value(cube_props).unwrap(),
		focus,
	};

	(region_str.to_string(), panel)
}

fn build_scene(scene_def: SceneDefinition) -> SceneConfig {
	let mut panels = HashMap::new();

	for panel_def in &scene_def.panels {
		let (key, panel) = build_panel(panel_def, scene_def.duration);
		panels.insert(key, panel);
	}

	SceneConfig {
		scene_name: scene_def.name.clone(),
		duration: scene_def.duration,
		start_time: 0,
		ui: vec![UILayoutIntent { panels }],
	}
}

fn interactive_scene_builder() -> Result<Vec<SceneDefinition>, Box<dyn std::error::Error>> {
	let mut scenes = Vec::new();

	let num_scenes: u32 = CustomType::new("How many scenes do you want to generate?")
		.with_default(1)
		.with_error_message("Please enter a valid number")
		.prompt()?;

	for scene_idx in 0..num_scenes {
		println!("\n🎬 Configuring Scene #{}", scene_idx + 1);

		let scene_name = Text::new(&format!("Scene name (scene_{})?", scene_idx + 1))
			.with_default(&format!("scene_{}", scene_idx + 1))
			.prompt()?;

		let duration: u64 = CustomType::new("Scene duration (ms)?")
			.with_default(30_000)
			.with_error_message("Please enter a valid duration in milliseconds")
			.prompt()?;

		let mut panels = Vec::new();

		// Ask which panels to include
		println!("\n📦 Select panels to include:");
		let available_regions = Region::all();

		for region in available_regions {
			let include = Confirm::new(&format!("Include {} panel?", region)).with_default(true).prompt()?;

			if !include {
				continue;
			}

			let children_count: usize = CustomType::new(&format!("  How many children for {} panel?", region))
				.with_default(0)
				.with_error_message("Please enter a valid number")
				.prompt()?;

			let has_focus = if matches!(region, Region::Video | Region::MainContent) {
				Confirm::new(&format!("  Apply focus to {} panel?", region)).with_default(false).prompt()?
			} else {
				false
			};

			let focus_intensity = if has_focus {
				let intensity: f64 = CustomType::new(&format!("  Focus intensity (0.0-1.0)?"))
					.with_default(0.7)
					.with_error_message("Please enter a value between 0.0 and 1.0")
					.prompt()?;
				intensity.clamp(0.0, 1.0)
			} else {
				0.0
			};

			panels.push(PanelDefinition {
				region,
				children_count,
				has_focus,
				focus_intensity,
			});
		}

		scenes.push(SceneDefinition {
			name: scene_name,
			duration,
			panels,
		});
	}

	Ok(scenes)
}

fn main() {
	println!("🎭 Scene Configuration Generator");
	println!("================================\n");

	let scene_definitions = match interactive_scene_builder() {
		Ok(defs) => defs,
		Err(e) => {
			eprintln!("❌ Error during configuration: {}", e);
			std::process::exit(1);
		}
	};

	let mut config_output = HashMap::new();

	for scene_def in scene_definitions {
		let scene_name = scene_def.name.clone();
		let scene_config = build_scene(scene_def);
		config_output.insert(scene_name, scene_config);
	}

	let output = ConfigOutput { scenes: config_output };

	let json_output = serde_json::to_string_pretty(&output).unwrap();

	let output_path = Text::new("Output file path?")
		.with_default("scenes.json")
		.prompt()
		.unwrap_or_else(|_| "scenes.json".to_string());

	match fs::write(&output_path, json_output) {
		Ok(_) => {
			println!("\n✅ Scene configuration generated: {}", output_path);
			println!("\n📝 Next steps:");
			println!("   1. Open {} and review the structure", output_path);
			println!("   2. Replace 'TODO_REGISTRY_KEY' with actual component registry keys");
			println!("   3. Update props values as needed");
			println!("   4. Adjust durations and timing");
		}
		Err(e) => {
			eprintln!("❌ Failed to write output file: {}", e);
			std::process::exit(1);
		}
	}
}
