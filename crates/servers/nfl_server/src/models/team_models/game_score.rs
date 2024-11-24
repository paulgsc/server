use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct GameScore<T: Identifiable> {
	pub id: u32,
	pub game: T,
	pub home_quarter_pts: [u8; 4],
	pub away_quarter_pts: [u8; 4],
}
