use crate::handlers::pipeline::fetch_cached;
use crate::metrics::otel::OperationTimer;
use crate::{
	models::gsheet::{validate_range, Attribution, DataResponse, FromGSheet, GanttChapter, GanttSubChapter, HexData, Metadata, RangeQuery, VideoChapters},
	models::nfl_tennis::{NFLGameScores, SheetDataItem},
	AppState, FileHostError,
};
use axum::extract::{Path, Query, State};
use axum::Json;
use some_cache::DedupCacheError;
use std::{borrow::Cow, collections::HashMap};
use tracing::instrument;

#[axum::debug_handler]
#[instrument(name = "get_attributions", skip(state), fields(sheet_id = %id, otel.kind = "server"))]
pub async fn get_attributions(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<Attribution>>, FileHostError> {
	let range = extract_and_validate_range(q)?;
	let cache_key = format!("get_attributions_{}_{}", id, range);

	let (raw_data, _) = fetch_cached(&state, "get_attributions", &cache_key, || async {
		let _fetch_timer = OperationTimer::new("get_attributions", "fetch_data");
		fetch_sheet_data(state.clone(), &id, Some(&range))
			.await
			.map_err(|e| DedupCacheError::OperationError(e.to_string()))
	})
	.await?;

	let attributions = {
		let _transform_timer = OperationTimer::new("get_attributions", "transform_data");
		Attribution::from_gsheet(&raw_data, true)?
	};

	Ok(Json(attributions))
}

#[axum::debug_handler]
#[instrument(name = "get_video_chapters", skip(state), fields(sheet_id = %id, otel.kind = "server"))]
pub async fn get_video_chapters(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<VideoChapters>>, FileHostError> {
	let range = extract_and_validate_range(q)?;
	let cache_key = format!("get_video_chapters_{}_{}", id, range);

	let (raw_data, _) = fetch_cached(&state, "get_video_chapters", &cache_key, || async {
		let _fetch_timer = OperationTimer::new("get_video_chapters", "fetch_data");
		fetch_sheet_data(state.clone(), &id, Some(&range))
			.await
			.map_err(|e| DedupCacheError::OperationError(e.to_string()))
	})
	.await?;

	let video_chapters = {
		let _transform_timer = OperationTimer::new("get_video_chapters", "transform_data");
		VideoChapters::from_gsheet(&raw_data, true)?
	};

	Ok(Json(video_chapters))
}

#[axum::debug_handler]
#[instrument(name = "get_gantt", skip(state), fields(sheet_id = %id, otel.kind = "server"))]
pub async fn get_gantt(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<GanttChapter>>, FileHostError> {
	let range = extract_and_validate_range(q)?;
	let cache_key = format!("get_gantt_{}_{}", id, range);

	let (gantt_chapters, _) = fetch_cached(&state, "get_gantt", &cache_key, || async {
		let _fetch_timer = OperationTimer::new("get_gantt", "fetch_data");
		let raw_data = fetch_sheet_data(state.clone(), &id, Some(&range))
			.await
			.map_err(|e| DedupCacheError::OperationError(e.to_string()))?;

		let boxed: Box<[Box<[Cow<str>]>]> = {
			let _box_timer = OperationTimer::new("get_gantt", "box_transform");
			raw_data
				.into_iter()
				.map(|inner| inner.into_iter().map(Cow::Owned).collect::<Box<[_]>>())
				.collect::<Box<[_]>>()
		};

		let chapters = {
			let _gantt_timer = OperationTimer::new("get_gantt", "gantt_transform");
			naive_gantt_transform(boxed)
		};

		Ok(chapters)
	})
	.await?;

	Ok(Json(gantt_chapters))
}

#[axum::debug_handler]
#[instrument(name = "get_nfl_tennis", skip(state), fields(sheet_id = %id, otel.kind = "server"))]
pub async fn get_nfl_tennis(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<DataResponse<Vec<SheetDataItem>>>, FileHostError> {
	let cache_key = format!("get_nfl_tennis_{}", id);

	let (sheet_collection, _) = fetch_cached(&state, "get_nfl_tennis", &cache_key, || async {
		let _fetch_timer = OperationTimer::new("get_nfl_tennis", "retrieve_all_sheets_data");
		let sheet_data = state
			.external
			.gsheet_reader
			.retrieve_all_sheets_data(&id)
			.await
			.map_err(|e| DedupCacheError::OperationError(e.to_string()))?;

		let mut collection = Vec::new();

		for (sheet_name, data) in sheet_data {
			let scores = NFLGameScores::from_gsheet(&data, true).map_err(|e| DedupCacheError::OperationError(e.to_string()))?;
			let df = NFLGameScores::create_dataframe(scores).map_err(|e| DedupCacheError::OperationError(format!("dataframe error: {e}")))?;
			let standings = NFLGameScores::get_team_standings(&df).map_err(|e| DedupCacheError::OperationError(e.to_string()))?;

			let sheet_item = SheetDataItem { name: sheet_name, standings };
			collection.push(sheet_item);
		}

		Ok(collection)
	})
	.await?;

	Ok(Json(DataResponse {
		data: sheet_collection,
		metadata: Metadata {
			title: "NFL Tennis Scores".to_string(),
			description: None,
		},
	}))
}

#[axum::debug_handler]
#[instrument(name = "get_nfl_roster", skip(state), fields(sheet_id = %id, otel.kind = "server"))]
pub async fn get_nfl_roster(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Vec<HexData>>, FileHostError> {
	let cache_key = format!("get_nfl_roster_{}", id);

	let (roster, _) = fetch_cached(&state, "get_nfl_roster", &cache_key, || async {
		let _fetch_timer = OperationTimer::new("get_nfl_roster", "fetch_data");
		let raw_data = fetch_sheet_data(state.clone(), &id, None)
			.await
			.map_err(|e| DedupCacheError::OperationError(format!("dataframe error: {e}")))?;

		let boxed: Box<[Box<[Cow<str>]>]> = raw_data
			.into_iter()
			.map(|inner| inner.into_iter().map(Cow::Owned).collect::<Box<[_]>>())
			.collect::<Box<[_]>>();

		let roster = {
			let _transform_timer = OperationTimer::new("get_nfl_roster", "transform_data");
			naive_roster_transform(boxed)
		};

		Ok(roster)
	})
	.await?;

	Ok(Json(roster))
}

#[instrument(name = "fetch_sheet_data", skip(state), fields(sheet_id, otel.kind = "internal"))]
async fn fetch_sheet_data(state: AppState, sheet_id: &str, range: Option<&str>) -> Result<Vec<Vec<String>>, FileHostError> {
	let data = match range {
		Some(query) => {
			let _timer = OperationTimer::new("fetch_sheet_data", "read_data_with_query");
			state.external.gsheet_reader.read_data(sheet_id, query).await.map_err(|e| FileHostError::upstream(e))?
		}
		None => {
			let _timer = OperationTimer::new("fetch_sheet_data", "retrieve_all_sheets");
			let res = state
				.external
				.gsheet_reader
				.retrieve_all_sheets_data(sheet_id)
				.await
				.map_err(|e| FileHostError::upstream(e))?;
			let (_, data) = res.into_iter().next().ok_or(FileHostError::UnexpectedSinglePair)?;
			data
		}
	};

	Ok(data)
}

// Keep existing transformation functions unchanged
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

fn extract_and_validate_range(q: RangeQuery) -> Result<String, FileHostError> {
	let range = q.range.ok_or(some_cache::DedupCacheError::TypeMismatch("range is required".to_string()))?;
	if !validate_range(&range) {
		return Err(FileHostError::upstream(sdk::SheetError::InvalidRange(range.into())));
	}
	Ok(range)
}
