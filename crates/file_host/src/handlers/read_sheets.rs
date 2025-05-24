use crate::{
	models::gsheet::{validate_range, Attribution, DataResponse, FromGSheet, GanttChapter, GanttSubChapter, HexData, Metadata, RangeQuery, VideoChapters},
	models::nfl_tennis::{NFLGameScores, SheetDataItem},
	AppState, FileHostError,
};
use axum::extract::{Path, Query, State};
use axum::Json;
use sdk::ReadSheets;
use std::{borrow::Cow, collections::HashMap, sync::Arc};

#[axum::debug_handler]
pub async fn get_attributions(State(state): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<Attribution>>, FileHostError> {
	let cache_key = format!("get_attributions_{}", id);
	if let Some(cached_data) = state.cache_store.get_json(&cache_key).await? {
		log::info!("Cache hit for key: {}", &cache_key);
		let attributions = Attribution::from_gsheet(&cached_data, true)?;
		return Ok(Json(attributions));
	}

	let q = extract_and_validate_range(q)?;
	let data = refetch(&state, &id, Some(&q)).await?;

	let attributions = Attribution::from_gsheet(&data, true)?;

	if data.len() <= 100 {
		log::info!("Caching data for key: {}", &cache_key);
		state.cache_store.set_json(&cache_key, &data).await?;
	} else {
		log::info!("Data too large to cache (size: {})", data.len());
	}

	Ok(Json(attributions))
}

#[axum::debug_handler]
pub async fn get_video_chapters(State(state): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<VideoChapters>>, FileHostError> {
	let cache_key = format!("get_video_chapters_{}", id);
	if let Some(cached_data) = state.cache_store.get_json(&cache_key).await? {
		log::info!("Cache hit for key: {}", &cache_key);
		let attributions = VideoChapters::from_gsheet(&cached_data, true)?;
		return Ok(Json(attributions));
	}

	let q = extract_and_validate_range(q)?;
	let data = refetch(&state, &id, Some(&q)).await?;

	let attributions = VideoChapters::from_gsheet(&data, true)?;

	if data.len() <= 100 {
		log::info!("Caching data for key: {}", &cache_key);
		state.cache_store.set_json(&cache_key, &data).await?;
	} else {
		log::info!("Data too large to cache (size: {})", data.len());
	}

	Ok(Json(attributions))
}

#[axum::debug_handler]
pub async fn get_gantt(State(state): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<GanttChapter>>, FileHostError> {
	let cache_key = format!("get_gantt_{}", id);
	if let Some(cached_data) = state.cache_store.get_json(&cache_key).await? {
		log::info!("Cache hit for key: {}", &cache_key);
		return Ok(Json(cached_data));
	}

	let q = extract_and_validate_range(q)?;
	let data = refetch(&state, &id, Some(&q)).await?;

	let boxed: Box<[Box<[Cow<str>]>]> = data.into_iter().map(|inner| inner.into_iter().map(Cow::Owned).collect::<Box<[_]>>()).collect::<Box<[_]>>();

	let chapters = naive_gantt_transform(boxed);

	if chapters.len() <= 100 {
		log::info!("Caching data for key: {}", &cache_key);
		state.cache_store.set_json(&cache_key, &chapters).await?;
	} else {
		log::info!("Data too large to cache (size: {})", chapters.len());
	}

	Ok(Json(chapters))
}

fn naive_gantt_transform(data: Box<[Box<[Cow<str>]>]>) -> Vec<GanttChapter> {
	let mut h: HashMap<Box<str>, GanttChapter> = HashMap::new();

	for row in data.iter().skip(1) {
		if row.len() < 6 {
			log::error!("Row has less than 12 elements: {:?}", row);
			continue;
		}

		let c_id = row[0].trim_matches('"').to_string().into_boxed_str();

		let chapters = h.entry(c_id.clone()).or_insert(GanttChapter {
			id: c_id,
			title: row[1].trim_matches('"').to_string().into_boxed_str(),
			start_time: row[2].trim_matches('"').parse::<i16>().unwrap_or(0),
			end_time: row[3].trim_matches('"').parse::<i16>().unwrap_or(0),
			description: row[4].trim_matches('"').to_string().into_boxed_str(),
			color: row[5].trim_matches('"').to_string().into_boxed_str(),
			sub_chapters: Vec::new(),
		});

		if row.len() >= 12 {
			chapters.sub_chapters.push(GanttSubChapter {
				id: row[6].trim_matches('"').to_string().into_boxed_str(),
				title: row[7].trim_matches('"').to_string().into_boxed_str(),
				start_time: row[8].trim_matches('"').parse::<i16>().unwrap_or(0),
				end_time: row[9].trim_matches('"').parse::<i16>().unwrap_or(0),
				description: row[10].trim_matches('"').to_string().into_boxed_str(),
				color: row[11].trim_matches('"').to_string().into_boxed_str(),
			});
		}
	}

	h.into_values().collect()
}

#[axum::debug_handler]
pub async fn get_nfl_tennis(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<DataResponse<Vec<SheetDataItem>>>, FileHostError> {
	if let Some(cached_data) = state.cache_store.get_json(&id).await? {
		log::info!("Cache hit for key: {}", &id);
		return Ok(Json(DataResponse {
			data: cached_data,
			metadata: Metadata {
				title: "NFL Tennis Scores".to_string(),
				description: None,
			},
		}));
	}
	let secret_file = state.config.client_secret_file.clone();
	let use_email = state.config.email_service_url.clone().unwrap_or("".to_string());

	let reader = ReadSheets::new(use_email, secret_file)?;

	let sheet_data = reader.retrieve_all_sheets_data(&id).await?;

	let mut sheet_collection = Vec::new();

	for (sheet_name, data) in sheet_data {
		let scores = NFLGameScores::from_gsheet(&data, true)?;
		let df = NFLGameScores::create_dataframe(scores)?;
		let standings = NFLGameScores::get_team_standings(&df)?;

		let sheet_item = SheetDataItem { name: sheet_name, standings };

		sheet_collection.push(sheet_item);
	}

	if sheet_collection.len() <= 1000 {
		log::info!("Caching data for key: {}", &id);
		state.cache_store.set_json(&id, &sheet_collection).await?;
	} else {
		log::info!("Data too large to cache (size: {})", sheet_collection.len());
	}

	Ok(Json(DataResponse {
		data: sheet_collection,
		metadata: Metadata {
			title: "NFL Tennis Scores".to_string(),
			description: None,
		},
	}))
}

#[axum::debug_handler]
pub async fn get_nfl_roster(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<Vec<HexData>>, FileHostError> {
	let cache_key = format!("get_nfl_roster{}", id);
	if let Some(cached_data) = state.cache_store.get_json(&cache_key).await? {
		log::info!("Cache hit for key: {}", &cache_key);
		return Ok(Json(cached_data));
	}

	let data = refetch(&state, &id, None).await?;

	let boxed: Box<[Box<[Cow<str>]>]> = data.into_iter().map(|inner| inner.into_iter().map(Cow::Owned).collect::<Box<[_]>>()).collect::<Box<[_]>>();

	let roster = naive_roster_transform(boxed);

	if roster.len() <= 100 {
		log::info!("Caching data for key: {}", &cache_key);
		state.cache_store.set_json(&cache_key, &roster).await?;
	} else {
		log::info!("Data too large to cache (size: {})", roster.len());
	}

	Ok(Json(roster))
}

fn naive_roster_transform(data: Box<[Box<[Cow<str>]>]>) -> Vec<HexData> {
	log::debug!("stupid data! {:?}", data);
	data
		.iter()
		.skip(1)
		.filter_map(|row| match row.as_ref() {
			[id, jersey_number, name, position, draft_pick, label, weight, color, ..] => Some(HexData {
				id: id.trim_matches('"').parse().unwrap_or(0),
				jersey_number: jersey_number.trim_matches('"').to_string(),
				name: name.trim_matches('"').to_string(),
				position: position.trim_matches('"').to_string(),
				draft_pick: draft_pick.trim_matches('"').to_string(),
				label: label.trim_matches('"').to_string(),
				weight: weight.trim_matches('"').parse().unwrap_or(0.0),
				color: color.trim_matches('"').parse().unwrap_or(0),
			}),
			_ => {
				log::error!("Row has less than 6 elements: {:?}", row);
				None
			}
		})
		.collect()
}

async fn refetch(state: &Arc<AppState>, sheet_id: &str, q: Option<&str>) -> Result<Vec<Vec<String>>, FileHostError> {
	let secret_file = state.config.client_secret_file.clone();
	let use_email = state.config.email_service_url.clone().unwrap_or("".to_string());

	let reader = ReadSheets::new(use_email, secret_file)?;

	let data = match q {
		Some(query) => reader.read_data(sheet_id, query).await?,
		None => {
			let res = reader.retrieve_all_sheets_data(sheet_id).await?;
			let (_, v) = match res.into_iter().next() {
				Some(pair) => pair,
				None => return Err(FileHostError::UnexpectedSinglePair),
			};
			v
		}
	};

	Ok(data)
}

fn extract_and_validate_range(q: RangeQuery) -> Result<String, FileHostError> {
	let range = q.range.ok_or(FileHostError::InvalidData)?;
	if !validate_range(&range) {
		return Err(FileHostError::SheetError(sdk::SheetError::InvalidRange(range.into())));
	}
	Ok(range)
}
