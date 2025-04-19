use crate::{
	models::gsheet::{get_cell_value, parse_cell, FromGSheet},
	FileHostError, GSheetDeriveError,
};
use gsheet_derive::FromGSheet;
use polars::lazy::dsl::*;
use polars::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize)]
pub struct DataItem {
	name: String,
	value: f64,
}

#[derive(Serialize, Deserialize)]
pub struct SheetDataItem {
	pub name: String,
	pub standings: Vec<DataItem>,
}

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

impl NFLGameScores {
	pub fn create_dataframe(games: Vec<Self>) -> Result<DataFrame, FileHostError> {
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

	pub fn get_team_standings(df: &DataFrame) -> Result<Vec<DataItem>, FileHostError> {
		let df_opponents = df.clone().lazy().rename(
			["team", "q1", "q2", "q3", "q4"],
			["opponent", "opponent_q1", "opponent_q2", "opponent_q3", "opponent_q4"],
			false,
		);

		// Create quarter point calculations for each quarter using Polars' proper comparison methods
		let q1_points = when(col("q1").gt(col("opponent_q1")))
			.then(lit(0.25))
			.when(col("q1").eq(col("opponent_q1")))
			.then(lit(0.125))
			.otherwise(lit(0.0));

		let q2_points = when(col("q2").gt(col("opponent_q2")))
			.then(lit(0.25))
			.when(col("q2").eq(col("opponent_q2")))
			.then(lit(0.125))
			.otherwise(lit(0.0));

		let q3_points = when(col("q3").gt(col("opponent_q3")))
			.then(lit(0.25))
			.when(col("q3").eq(col("opponent_q3")))
			.then(lit(0.125))
			.otherwise(lit(0.0));

		let q4_points = when(col("q4").gt(col("opponent_q4")))
			.then(lit(0.25))
			.when(col("q4").eq(col("opponent_q4")))
			.then(lit(0.125))
			.otherwise(lit(0.0));

		// Sum all quarter points together
		let df_merged = df
			.clone()
			.lazy()
			.join(df_opponents, &[col("game_id")], &[col("game_id")], JoinType::Inner.into())
			.filter(col("team").neq(col("opponent")))
			.with_column((q1_points + q2_points + q3_points + q4_points).alias("quarter_points"))
			.collect()?;

		// Now quarter_points exists in df_merged
		let aggregated = df_merged
			.lazy()
			.group_by(["team"])
			.agg([col("quarter_points").sum().alias("total_quarter_points")])
			.sort(vec!["total_quarter_points"], Default::default())
			.collect()?;

		let teams: Vec<String> = aggregated.column("team")?.str()?.into_iter().filter_map(|s| s.map(|s| s.to_string())).collect();

		let scores: Vec<f64> = aggregated.column("total_quarter_points")?.f64()?.into_iter().map(|f| f.unwrap()).collect();

		let result: Vec<DataItem> = teams.into_iter().zip(scores.into_iter()).map(|(name, value)| DataItem { name, value }).collect();
		Ok(result)
	}
}
