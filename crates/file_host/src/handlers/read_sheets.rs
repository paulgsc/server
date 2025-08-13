use crate::{
	metrics::http::{CACHE_OPERATIONS, OPERATION_DURATION},
	models::gsheet::{validate_range, Attribution, DataResponse, FromGSheet, GanttChapter, GanttSubChapter, HexData, Metadata, RangeQuery, VideoChapters},
	models::nfl_tennis::{NFLGameScores, SheetDataItem},
	AppState, FileHostError,
};
use crate::{record_cache_op, timed_operation};
use axum::extract::{Path, Query, State};
use axum::Json;
use std::{borrow::Cow, collections::HashMap};
use tracing::instrument;

#[axum::debug_handler]
#[instrument(name = "get_attributions", skip(state), fields(sheet_id = %id))]
pub async fn get_attributions(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<Attribution>>, FileHostError> {
	let cache_key = format!("get_attributions_{}", id);

	let cache_result = timed_operation!("get_attributions", "cached_check", true, { state.cache_store.get(&cache_key).await })?;

	if let Some(cached_data) = cache_result {
		record_cache_op!("get_attributions", "get", "hit");

		let attributions = timed_operation!("get_attributions", "deserialzie_cache", true, { Attribution::from_gsheet(&cached_data, true) })?;

		return Ok(Json(attributions));
	}

	record_cache_op!("get_attributions", "get", "miss");
	let q = timed_operation!("get_attributions", "validate_range", false, { extract_and_validate_range(q) })?;

	let data = timed_operation!("get_attributions", "fetch_data", false, { refetch(state.clone(), &id, Some(&q)).await })?;

	let attributions = timed_operation!("get_attributions_", "tranform_data", false, { Attribution::from_gsheet(&data, true) })?;

	if data.len() <= 100 {
		timed_operation!("get_attributions", "cache_set", false, {
			async {
				match state.cache_store.set(&cache_key, &data, None).await {
					Ok(_) => {
						record_cache_op!("get_attributions", "set", "success");
					}
					Err(e) => {
						record_cache_op!("get_attributions", "set", "error");
						tracing::error!("Failed to cache data: {}", e);
					}
				}
			}
		})
		.await;
	} else {
		tracing::warn!("Data too large to cache (size: {})", data.len());
	}

	Ok(Json(attributions))
}

#[axum::debug_handler]
#[instrument(name = "get_video_chapters", skip(state), fields(sheet_id = %id))]
pub async fn get_video_chapters(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<VideoChapters>>, FileHostError> {
	let cache_key = format!("get_video_chapters_{}", id);

	let cache_result = timed_operation!("get_video_chapters", "cached_check", true, { state.cache_store.get(&cache_key).await })?;

	if let Some(cached_data) = cache_result {
		record_cache_op!("get_video_chapters", "get", "hit");

		let video_chapters = timed_operation!("get_video_chapters", "deserialize_cache", true, { VideoChapters::from_gsheet(&cached_data, true) })?;

		return Ok(Json(video_chapters));
	}

	record_cache_op!("get_video_chapters", "get", "miss");

	let q = timed_operation!("get_video_chapters", "validate_range", false, { extract_and_validate_range(q) })?;

	let data = timed_operation!("get_video_chapters", "fetch_data", false, { refetch(state.clone(), &id, Some(&q)).await })?;

	let video_chapters = timed_operation!("get_video_chapters", "transform_data", false, { VideoChapters::from_gsheet(&data, true) })?;

	if data.len() <= 100 {
		timed_operation!("get_video_chapters", "cache_set", false, {
			async {
				match state.cache_store.set(&cache_key, &data, None).await {
					Ok(_) => record_cache_op!("get_video_chapters", "set", "success"),
					Err(_) => record_cache_op!("get_video_chapters", "set", "error"),
				}
			}
		})
		.await;
	}

	Ok(Json(video_chapters))
}

#[axum::debug_handler]
#[instrument(name = "get_gantt", skip(state), fields(sheet_id = %id))]
pub async fn get_gantt(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<GanttChapter>>, FileHostError> {
	let cache_key = format!("get_gantt_{}", id);

	let cache_result = timed_operation!("get_gantt", "cache_check", true, { state.cache_store.get(&cache_key).await })?;

	if let Some(cached_data) = cache_result {
		record_cache_op!("get_gantt", "get", "hit");
		return Ok(Json(cached_data));
	}

	record_cache_op!("get_gantt", "get", "miss");

	let q = timed_operation!("get_gantt", "validate_range", false, { extract_and_validate_range(q) })?;

	let data = timed_operation!("get_gantt", "fetch_data", false, { refetch(state.clone(), &id, Some(&q)).await })?;

	let boxed: Box<[Box<[Cow<str>]>]> = timed_operation!("get_gantt", "box_transform", false, {
		data.into_iter().map(|inner| inner.into_iter().map(Cow::Owned).collect::<Box<[_]>>()).collect::<Box<[_]>>()
	});

	let chapters = timed_operation!("get_gantt", "gantt_transform", false, { naive_gantt_transform(boxed) });

	if chapters.len() <= 100 {
		timed_operation!("get_gantt", "cache_set", false, {
			async {
				match state.cache_store.set(&cache_key, &chapters, None).await {
					Ok(_) => record_cache_op!("get_gantt", "set", "success"),
					Err(_) => record_cache_op!("get_gantt", "set", "error"),
				}
			}
		})
		.await;
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
#[instrument(name = "get_nfl_tennis", skip(state), fields(sheet_id = %id))]
pub async fn get_nfl_tennis(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<DataResponse<Vec<SheetDataItem>>>, FileHostError> {
	let cache_result = timed_operation!("get_nfl_tennis", "cached_check", true, { state.cache_store.get(&id).await })?;

	if let Some(cached_data) = cache_result {
		record_cache_op!("get_nfl_tennis", "get", "hit");
		return Ok(Json(DataResponse {
			data: cached_data,
			metadata: Metadata {
				title: "NFL Tennis Scores".to_string(),
				description: None,
			},
		}));
	}

	record_cache_op!("get_nfl_tennis", "get", "miss");

	let sheet_data = timed_operation!("get_nfl_tennis", "retrieve_all_sheets_data", false, {
		state.gsheet_reader.retrieve_all_sheets_data(&id).await
	})?;

	let mut sheet_collection = Vec::new();

	for (sheet_name, data) in sheet_data {
		let scores = NFLGameScores::from_gsheet(&data, true)?;
		let df = NFLGameScores::create_dataframe(scores)?;
		let standings = NFLGameScores::get_team_standings(&df)?;

		let sheet_item = SheetDataItem { name: sheet_name, standings };

		sheet_collection.push(sheet_item);
	}

	if sheet_collection.len() <= 1000 {
		timed_operation!("get_nfl_tennis", "cache_set", false, {
			async {
				match state.cache_store.set(&id, &sheet_collection, None).await {
					Ok(_) => {
						record_cache_op!("get_nfl_tennis", "set", "success");
					}
					Err(e) => {
						record_cache_op!("get_nfl_tennis", "set", "error");
						tracing::error!("Failed to cache data: {}", e);
					}
				}
			}
		})
		.await;
	} else {
		tracing::info!("Data too large to cache (size: {})", sheet_collection.len());
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
#[instrument(name = "get_nfl_roster", skip(state), fields(sheet_id = %id))]
pub async fn get_nfl_roster(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Vec<HexData>>, FileHostError> {
	let cache_key = format!("get_nfl_roster{}", id);

	let cache_result = timed_operation!("get_nfl_roster", "cached_check", true, { state.cache_store.get(&cache_key).await })?;

	if let Some(cached_data) = cache_result {
		record_cache_op!("get_nfl_roster", "get", "hit");
		return Ok(Json(cached_data));
	}

	record_cache_op!("get_nfl_roster", "get", "miss");

	let data = timed_operation!("get_nfl_roster", "fetch_data", false, { refetch(state.clone(), &id, None).await })?;

	let boxed: Box<[Box<[Cow<str>]>]> = data.into_iter().map(|inner| inner.into_iter().map(Cow::Owned).collect::<Box<[_]>>()).collect::<Box<[_]>>();

	let roster = timed_operation!("get_nfl_roster", "transform_data", false, { naive_roster_transform(boxed) });

	if roster.len() <= 100 {
		timed_operation!("get_nfl_roster", "cache_set", false, {
			async {
				match state.cache_store.set(&cache_key, &roster, None).await {
					Ok(_) => {
						record_cache_op!("get_nfl_roster", "set", "success");
					}
					Err(e) => {
						record_cache_op!("get_nfl_roster", "set", "error");
						tracing::error!("Failed to cache data: {}", e);
					}
				}
			}
		})
		.await;
	} else {
		tracing::info!("Data too large to cache (size: {})", roster.len());
	}

	Ok(Json(roster))
}

fn naive_roster_transform(data: Box<[Box<[Cow<str>]>]>) -> Vec<HexData> {
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

#[instrument(name = "refetch", skip(state), fields(sheet_id))]
async fn refetch(state: AppState, sheet_id: &str, q: Option<&str>) -> Result<Vec<Vec<String>>, FileHostError> {
	let data = match q {
		Some(query) => timed_operation!("refetch", "read_data_with_query", false, { state.gsheet_reader.read_data(sheet_id, query).await })?,
		None => {
			let res = timed_operation!("refetch", "retrieve_all_sheets", false, { state.gsheet_reader.retrieve_all_sheets_data(sheet_id).await })?;
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
