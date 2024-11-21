#[derive(Debug, Clone)]
pub struct Location {
	pub state: State,
	pub city: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StadiumType {
	Indoor,
	Outdoor,
	Retractable,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SurfaceType {
	Grass,
	AstroTurf,
	Hybrid,
}

#[derive(Debug, Clone)]
pub struct Stadium {
	pub id: u32,
	pub name: String,
	pub location: Location,
	pub stadium_type: StadiumType,
	pub surface_type: SurfaceType,
	pub capacity: u32,
}
