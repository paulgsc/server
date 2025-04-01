use crate::{
	models::gsheet::{get_cell_value, parse_cell, FromGSheet},
	FileHostError, GSheetDeriveError,
};
use gsheet_derive::FromGSheet;
use polars::lazy::dsl::*;
use polars::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, FromGSheet)]
pub struct NFLGameScores {
	#[gsheet(column = "A")]
	game_id: i32,
	#[gsheet(column = "B")]
	team: String,
	#[gsheet(column = "C")]
	home_away: String,
	#[gsheet(column = "D")]
	date: String,
	#[gsheet(column = "E")]
	q1: i32,
	#[gsheet(column = "F")]
	q2: i32,
	#[gsheet(column = "G")]
	q3: i32,
	#[gsheet(column = "H")]
	q4: i32,
	#[gsheet(column = "I")]
	ot: i32,
	#[gsheet(column = "J")]
	total: i32,
}

fn calculate_quarter_points(team_q: &[i32], opponent_q: &[i32]) -> f32 {
	let mut points = 0.0;
	for (t, o) in team_q.iter().zip(opponent_q.iter()) {
		points += if t > o {
			0.25
		} else if t == o {
			0.125
		} else {
			0.0
		};
	}
	points
}

pub fn create_dataframe(games: Vec<NFLGameScores>) -> Result<DataFrame, FileHostError> {
	let df = DataFrame::new(vec![
		Series::new("game_id".into(), games.iter().map(|g| g.game_id).collect::<Vec<_>>()).into(),
		Series::new("team".into(), games.iter().map(|g| g.team.as_str()).collect::<Vec<_>>()).into(),
		Series::new("home_away".into(), games.iter().map(|g| g.home_away.as_str()).collect::<Vec<_>>()).into(),
		Series::new("date".into(), games.iter().map(|g| g.date.as_str()).collect::<Vec<_>>()).into(),
		Series::new("q1".into(), games.iter().map(|g| g.q1).collect::<Vec<_>>()).into(),
		Series::new("q2".into(), games.iter().map(|g| g.q2).collect::<Vec<_>>()).into(),
		Series::new("q3".into(), games.iter().map(|g| g.q3).collect::<Vec<_>>()).into(),
		Series::new("q4".into(), games.iter().map(|g| g.q4).collect::<Vec<_>>()).into(),
		Series::new("ot".into(), games.iter().map(|g| g.ot).collect::<Vec<_>>()).into(),
		Series::new("total".into(), games.iter().map(|g| g.total).collect::<Vec<_>>()).into(),
	])?;
	Ok(df)
}

pub fn get_team_standings(df: &DataFrame) -> Result<Vec<(String, f32)>, FileHostError> {
	let df_opponents = df.clone().lazy().rename(
		["team", "q1", "q2", "q3", "q4"],
		["opponent", "opponent_q1", "opponent_q2", "opponent_q3", "opponent_q4"],
		false,
	);

	let df_merged = df
		.clone()
		.lazy()
		.join(df_opponents, &[col("game_id")], &[col("game_id")], JoinType::Inner.into())
		.filter(col("team").neq(col("opponent"))) // Ensure no self-matching
		.with_column(map_multiple(
			|columns: &mut [Column]| {
				let series: Vec<Series> = columns
					.iter_mut()
					.map(|col| col.as_series())
					.filter_map(|opt_series| opt_series.map(|series| series.clone()))
					.collect();

				let team_q: Vec<i32> = series[..4].iter().flat_map(|s| s.i32().unwrap().into_iter().flatten()).collect();

				let opponent_q: Vec<i32> = series[4..].iter().flat_map(|s| s.i32().unwrap().into_iter().flatten()).collect();

				let result_series = Series::new("quarter_points".into(), vec![calculate_quarter_points(&team_q, &opponent_q)]);

				Ok(Some(result_series.into()))
			},
			[
				col("q1"),
				col("q2"),
				col("q3"),
				col("q4"),
				col("opponent_q1"),
				col("opponent_q2"),
				col("opponent_q3"),
				col("opponent_q4"),
			],
			GetOutput::from_type(DataType::Float32),
		))
		.collect()?;

	// Aggregate by team
	let aggregated = df_merged
		.lazy()
		.group_by(["team"])
		.agg([col("quarter_points").sum().alias("total_quarter_points")])
		.sort(vec!["total_quarter_points"], Default::default()) 
		.collect()?;

	let teams: Vec<String> = aggregated
		.column("team")?
		.str()? // Use `.str()` instead of `.utf8()`
		.into_iter()
		.filter_map(|s| s.map(|s| s.to_string()))
		.collect();
	let scores: Vec<f32> = aggregated.column("total_quarter_points")?.f32()?.into_iter().map(|f| f.unwrap()).collect();

	Ok(teams.into_iter().zip(scores.into_iter()).collect())
}
