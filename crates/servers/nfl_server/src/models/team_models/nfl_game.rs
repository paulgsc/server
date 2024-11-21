trait Identifiable {
	fn id(&self) -> u32;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WeatherCondition {
	Sunny,
	Cloudy,
	Rainy,
	Snowy,
	Windy,
	Foggy,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DayNight {
	Day,
	Night,
}

#[derive(Debug, Clone)]
pub struct Weather {
	pub id: u32,
	pub condition: WeatherCondition,
	pub day_night: DayNight,
	pub temperature: f32,
	pub wind_speed: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct Team<T: Identifiable, S: Identifiable, M: Identifiable> {
	pub id: u32,
	pub abbreviation: &S,
	pub name: &T,
	pub mascot: &M,
	pub home_stadium_id: u32,
}

#[derive(Debug, Clone)]
pub struct NFLGame<T: Identifiable, S: Identifiable, W: Identifiable> {
	pub id: u32,
	pub date: String,
	pub home_team: &T,
	pub away_team: &T,
	pub stadium: &S,
	pub weather: &W,
}
