use crate::{
	models::gsheet::{validate_range, Attribution, FromGSheet, GanttChapter, GanttSubChapter, RangeQuery, VideoChapters},
	AppState, FileHostError,
};
use axum::extract::{Path, Query, State};
use axum::Json;
use sdk::ReadSheets;
use std::{borrow::Cow, collections::HashMap, sync::Arc};

#[axum::debug_handler]
pub async fn get_attributions(State(state): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<Attribution>>, FileHostError> {
	if let Some(cached_data) = state.cache_store.get_json(&id).await? {
		log::info!("Cache hit for key: {}", &id);
		let attributions = Attribution::from_gsheet(&cached_data, true)?;
		return Ok(Json(attributions));
	}

	let q = extract_and_validate_range(q)?;
	let data = refetch(&state, &id, &q).await?;

	let attributions = Attribution::from_gsheet(&data, true)?;

	if data.len() <= 100 {
		log::info!("Caching data for key: {}", &id);
		state.cache_store.set_json(&id, &data).await?;
	} else {
		log::info!("Data too large to cache (size: {})", data.len());
	}

	Ok(Json(attributions))
}

#[axum::debug_handler]
pub async fn get_video_chapters(State(state): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<VideoChapters>>, FileHostError> {
	if let Some(cached_data) = state.cache_store.get_json(&id).await? {
		log::info!("Cache hit for key: {}", &id);
		let attributions = VideoChapters::from_gsheet(&cached_data, true)?;
		return Ok(Json(attributions));
	}

	let q = extract_and_validate_range(q)?;
	let data = refetch(&state, &id, &q).await?;

	let attributions = VideoChapters::from_gsheet(&data, true)?;

	if data.len() <= 100 {
		log::info!("Caching data for key: {}", &id);
		state.cache_store.set_json(&id, &data).await?;
	} else {
		log::info!("Data too large to cache (size: {})", data.len());
	}

	Ok(Json(attributions))
}

#[axum::debug_handler]
pub async fn get_gantt(State(state): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<GanttChapter>>, FileHostError> {
	if let Some(cached_data) = state.cache_store.get_json(&id).await? {
		log::info!("Cache hit for key: {}", &id);
		return Ok(Json(cached_data));
	}

	let q = extract_and_validate_range(q)?;
	let data = refetch(&state, &id, &q).await?;

	let boxed: Box<[Box<[Cow<str>]>]> = data.into_iter().map(|inner| inner.into_iter().map(Cow::Owned).collect::<Box<[_]>>()).collect::<Box<[_]>>();

	let chapters = naive_gantt_transform(boxed);

	if chapters.len() <= 100 {
		log::info!("Caching data for key: {}", &id);
		state.cache_store.set_json(&id, &chapters).await?;
	} else {
		log::info!("Data too large to cache (size: {})", chapters.len());
	}

	Ok(Json(chapters))
}

fn naive_gantt_transform(data: Box<[Box<[Cow<str>]>]>) -> Vec<GanttChapter> {
	let mut h: HashMap<Box<str>, GanttChapter> = HashMap::new();

	for row in data.iter().skip(1) {
		let c_id = row[0].trim_matches('"').to_string().into_boxed_str();

		let chapters = h.entry(c_id.clone()).or_insert(GanttChapter {
			id: c_id,
			title: row[1].trim_matches('"').to_string().into_boxed_str(),
			start_time: row[2].trim_matches('"').to_string().into_boxed_str(),
			end_time: row[3].trim_matches('"').to_string().into_boxed_str(),
			description: row[4].trim_matches('"').to_string().into_boxed_str(),
			color: row[5].trim_matches('"').to_string().into_boxed_str(),
			sub_chapters: Vec::new(),
		});

		chapters.sub_chapters.push(GanttSubChapter {
			id: row[6].trim_matches('"').to_string().into_boxed_str(),
			title: row[7].trim_matches('"').to_string().into_boxed_str(),
			start_time: row[8].trim_matches('"').to_string().into_boxed_str(),
			end_time: row[9].trim_matches('"').to_string().into_boxed_str(),
			description: row[10].trim_matches('"').to_string().into_boxed_str(),
			color: row[11].trim_matches('"').to_string().into_boxed_str(),
		});
	}

	h.into_values().collect()
}

async fn refetch(state: &Arc<AppState>, sheet_id: &str, q: &str) -> Result<Vec<Vec<String>>, FileHostError> {
	let secret_file = state.config.client_secret_file.clone();
	let use_email = state.config.email_service_url.clone().unwrap_or("".to_string());

	let reader = ReadSheets::new(use_email, secret_file)?;

	let data = reader.read_data(&sheet_id, q).await?;

	Ok(data)
}

fn extract_and_validate_range(q: RangeQuery) -> Result<String, FileHostError> {
	let range = q.range.ok_or(FileHostError::InvalidData)?;
	if !validate_range(&range) {
		return Err(FileHostError::SheetError(sdk::SheetError::InvalidRange(range.into())));
	}
	Ok(range)
}
