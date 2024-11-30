use crate::common::nfl_server_error::NflServerError as Error;
use nest::http::Error as NestError;
use std::convert::TryFrom;
use std::str::FromStr;

#[derive(Debug)]
pub enum TeamAbbreviation {
	ARI, // Arizona Cardinals
	ATL, // Atlanta Falcons
	BAL, // Baltimore Ravens
	BUF, // Buffalo Bills
	CAR, // Carolina Panthers
	CHI, // Chicago Bears
	CIN, // Cincinnati Bengals
	CLE, // Cleveland Browns
	DAL, // Dallas Cowboys
	DEN, // Denver Broncos
	DET, // Detroit Lions
	GB,  // Green Bay Packers
	HOU, // Houston Texans
	IND, // Indianapolis Colts
	JAC, // Jacksonville Jaguars
	KC,  // Kansas City Chiefs
	LV,  // Las Vegas Raiders
	LAC, // Los Angeles Chargers
	LAR, // Los Angeles Rams
	MIA, // Miami Dolphins
	MIN, // Minnesota Vikings
	NE,  // New England Patriots
	NO,  // New Orleans Saints
	NYG, // New York Giants
	NYJ, // New York Jets
	PHI, // Philadelphia Eagles
	PIT, // Pittsburgh Steelers
	SEA, // Seattle Seahawks
	SF,  // San Francisco 49ers
	TB,  // Tampa Bay Buccaneers
	TEN, // Tennessee Titans
	WAS, // Washington Commanders
}

#[derive(Debug)]
pub enum TeamName {
	ArizonaCardinals,
	AtlantaFalcons,
	BaltimoreRavens,
	BuffaloBills,
	CarolinaPanthers,
	ChicagoBears,
	CincinnatiBengals,
	ClevelandBrowns,
	DallasCowboys,
	DenverBroncos,
	DetroitLions,
	GreenBayPackers,
	HoustonTexans,
	IndianapolisColts,
	JacksonvilleJaguars,
	KansasCityChiefs,
	LasVegasRaiders,
	LosAngelesChargers,
	LosAngelesRams,
	MiamiDolphins,
	MinnesotaVikings,
	NewEnglandPatriots,
	NewOrleansSaints,
	NewYorkGiants,
	NewYorkJets,
	PhiladelphiaEagles,
	PittsburghSteelers,
	SanFranciscoFortyNiners,
	SeattleSeahawks,
	TampaBayBuccaneers,
	TennesseeTitans,
	WashingtonCommanders,
}

pub enum TeamId {
	ARI = 1,
	ATL = 2,
	BAL = 3,
	BUF = 4,
	CAR = 5,
	CHI = 6,
	CIN = 7,
	CLE = 8,
	DAL = 9,
	DEN = 10,
	DET = 11,
	GB = 12,
	HOU = 13,
	IND = 14,
	JAX = 15,
	KC = 16,
	MIA = 17,
	MIN = 18,
	NE = 19,
	NO = 20,
	NYG = 21,
	NYJ = 22,
	LV = 23,
	PHI = 24,
	PIT = 25,
	LAC = 26,
	SF = 27,
	SEA = 28,
	LAR = 29,
	TB = 30,
	TEN = 31,
	WAS = 32,
}

impl From<TeamId> for u32 {
	fn from(team_id: TeamId) -> u32 {
		match team_id {
			TeamId::ARI => 1,
			TeamId::ATL => 2,
			TeamId::BAL => 3,
			TeamId::BUF => 4,
			TeamId::CAR => 5,
			TeamId::CHI => 6,
			TeamId::CIN => 7,
			TeamId::CLE => 8,
			TeamId::DAL => 9,
			TeamId::DEN => 10,
			TeamId::DET => 11,
			TeamId::GB => 12,
			TeamId::HOU => 13,
			TeamId::IND => 14,
			TeamId::JAX => 15,
			TeamId::KC => 16,
			TeamId::MIA => 17,
			TeamId::MIN => 18,
			TeamId::NE => 19,
			TeamId::NO => 20,
			TeamId::NYG => 21,
			TeamId::NYJ => 22,
			TeamId::PHI => 23,
			TeamId::PIT => 24,
			TeamId::LAC => 25,
			TeamId::SF => 26,
			TeamId::SEA => 27,
			TeamId::LV => 28,
			TeamId::LAR => 29,
			TeamId::TB => 30,
			TeamId::TEN => 31,
			TeamId::WAS => 32,
		}
	}
}

impl TryFrom<u32> for TeamId {
	type Error = &'static str;

	fn try_from(value: u32) -> Result<Self, Self::Error> {
		match value {
			1 => Ok(TeamId::ARI),
			2 => Ok(TeamId::ATL),
			3 => Ok(TeamId::BAL),
			4 => Ok(TeamId::BUF),
			5 => Ok(TeamId::CAR),
			6 => Ok(TeamId::CHI),
			7 => Ok(TeamId::CIN),
			8 => Ok(TeamId::CLE),
			9 => Ok(TeamId::DAL),
			10 => Ok(TeamId::DEN),
			11 => Ok(TeamId::DET),
			12 => Ok(TeamId::GB),
			13 => Ok(TeamId::HOU),
			14 => Ok(TeamId::IND),
			15 => Ok(TeamId::JAX),
			16 => Ok(TeamId::KC),
			17 => Ok(TeamId::MIA),
			18 => Ok(TeamId::MIN),
			19 => Ok(TeamId::NE),
			20 => Ok(TeamId::NO),
			21 => Ok(TeamId::NYG),
			22 => Ok(TeamId::NYJ),
			23 => Ok(TeamId::LV),
			24 => Ok(TeamId::PHI),
			25 => Ok(TeamId::PIT),
			26 => Ok(TeamId::LAC),
			27 => Ok(TeamId::SF),
			28 => Ok(TeamId::SEA),
			29 => Ok(TeamId::LAR),
			30 => Ok(TeamId::TB),
			31 => Ok(TeamId::TEN),
			32 => Ok(TeamId::WAS),
			_ => Err("Invalid Team ID"),
		}
	}
}

impl FromStr for TeamId {
	type Err = Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase().as_str() {
			"cardinals" | "arizona" | "ari" => Ok(TeamId::ARI),
			"falcons" | "atlanta" | "atl" => Ok(TeamId::ATL),
			"ravens" | "baltimore" | "bal" => Ok(TeamId::BAL),
			"bills" | "buffalo" | "buf" => Ok(TeamId::BUF),
			"panthers" | "carolina" | "car" => Ok(TeamId::CAR),
			"bears" | "chicago" | "chi" => Ok(TeamId::CHI),
			"bengals" | "cincinnati" | "cin" => Ok(TeamId::CIN),
			"browns" | "cleveland" | "cle" => Ok(TeamId::CLE),
			"cowboys" | "dallas" | "dal" => Ok(TeamId::DAL),
			"broncos" | "denver" | "den" => Ok(TeamId::DEN),
			"lions" | "detroit" | "det" => Ok(TeamId::DET),
			"packers" | "green bay" | "gb" => Ok(TeamId::GB),
			"texans" | "houston" | "hou" => Ok(TeamId::HOU),
			"colts" | "indianapolis" | "ind" => Ok(TeamId::IND),
			"jaguars" | "jacksonville" | "jax" => Ok(TeamId::JAX),
			"chiefs" | "kansas city" | "kc" => Ok(TeamId::KC),
			"dolphins" | "miami" | "mia" => Ok(TeamId::MIA),
			"vikings" | "minnesota" | "min" => Ok(TeamId::MIN),
			"patriots" | "new england" | "ne" => Ok(TeamId::NE),
			"saints" | "new orleans" | "no" => Ok(TeamId::NO),
			"giants" | "new york giants" | "nyg" => Ok(TeamId::NYG),
			"jets" | "new york jets" | "nyj" => Ok(TeamId::NYJ),
			"eagles" | "philadelphia" | "phi" => Ok(TeamId::PHI),
			"steelers" | "pittsburgh" | "pit" => Ok(TeamId::PIT),
			"chargers" | "los angeles chargers" | "lac" => Ok(TeamId::LAC),
			"49ers" | "san francisco" | "sf" => Ok(TeamId::SF),
			"seahawks" | "seattle" | "sea" => Ok(TeamId::SEA),
			"raiders" | "las vegas" | "lv" => Ok(TeamId::LV),
			"rams" | "los angeles rams" | "lar" => Ok(TeamId::LAR),
			"buccaneers" | "tampa bay" | "tb" => Ok(TeamId::TB),
			"titans" | "tennessee" | "ten" => Ok(TeamId::TEN),
			"commanders" | "washington" | "was" => Ok(TeamId::WAS),
			_ => Err(Error::NestError(NestError::unprocessable_entity(vec![("team id", "Invalid Team ID")]))),
		}
	}
}

#[derive(Debug)]
pub struct TeamNameMeta {
	pub id: u32,
	pub name: TeamName,
	pub abbreviation: TeamAbbreviation,
}

impl FromStr for TeamNameMeta {
	type Err = Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let team_id = TeamId::from_str(s)?;
		Self::try_from(u32::from(team_id))
	}
}

impl TryFrom<u32> for TeamNameMeta {
	type Error = Error;

	fn try_from(id: u32) -> Result<Self, Error> {
		match TeamId::try_from(id) {
			Ok(TeamId::ARI) => Ok(Self {
				id,
				name: TeamName::ArizonaCardinals,
				abbreviation: TeamAbbreviation::ARI,
			}),
			Ok(TeamId::ATL) => Ok(Self {
				id,
				name: TeamName::AtlantaFalcons,
				abbreviation: TeamAbbreviation::ATL,
			}),
			Ok(TeamId::BAL) => Ok(Self {
				id,
				name: TeamName::BaltimoreRavens,
				abbreviation: TeamAbbreviation::BAL,
			}),
			Ok(TeamId::BUF) => Ok(Self {
				id,
				name: TeamName::BuffaloBills,
				abbreviation: TeamAbbreviation::BUF,
			}),
			Ok(TeamId::CAR) => Ok(Self {
				id,
				name: TeamName::CarolinaPanthers,
				abbreviation: TeamAbbreviation::CAR,
			}),
			Ok(TeamId::CHI) => Ok(Self {
				id,
				name: TeamName::ChicagoBears,
				abbreviation: TeamAbbreviation::CHI,
			}),
			Ok(TeamId::CIN) => Ok(Self {
				id,
				name: TeamName::CincinnatiBengals,
				abbreviation: TeamAbbreviation::CIN,
			}),
			Ok(TeamId::CLE) => Ok(Self {
				id,
				name: TeamName::ClevelandBrowns,
				abbreviation: TeamAbbreviation::CLE,
			}),
			Ok(TeamId::DAL) => Ok(Self {
				id,
				name: TeamName::DallasCowboys,
				abbreviation: TeamAbbreviation::DAL,
			}),
			Ok(TeamId::DEN) => Ok(Self {
				id,
				name: TeamName::DenverBroncos,
				abbreviation: TeamAbbreviation::DEN,
			}),
			Ok(TeamId::DET) => Ok(Self {
				id,
				name: TeamName::DetroitLions,
				abbreviation: TeamAbbreviation::DET,
			}),
			Ok(TeamId::GB) => Ok(Self {
				id,
				name: TeamName::GreenBayPackers,
				abbreviation: TeamAbbreviation::GB,
			}),
			Ok(TeamId::HOU) => Ok(Self {
				id,
				name: TeamName::HoustonTexans,
				abbreviation: TeamAbbreviation::HOU,
			}),
			Ok(TeamId::IND) => Ok(Self {
				id,
				name: TeamName::IndianapolisColts,
				abbreviation: TeamAbbreviation::IND,
			}),
			Ok(TeamId::JAX) => Ok(Self {
				id,
				name: TeamName::JacksonvilleJaguars,
				abbreviation: TeamAbbreviation::JAC,
			}),
			Ok(TeamId::KC) => Ok(Self {
				id,
				name: TeamName::KansasCityChiefs,
				abbreviation: TeamAbbreviation::KC,
			}),
			Ok(TeamId::MIA) => Ok(Self {
				id,
				name: TeamName::MiamiDolphins,
				abbreviation: TeamAbbreviation::MIA,
			}),
			Ok(TeamId::MIN) => Ok(Self {
				id,
				name: TeamName::MinnesotaVikings,
				abbreviation: TeamAbbreviation::MIN,
			}),
			Ok(TeamId::NE) => Ok(Self {
				id,
				name: TeamName::NewEnglandPatriots,
				abbreviation: TeamAbbreviation::NE,
			}),
			Ok(TeamId::NO) => Ok(Self {
				id,
				name: TeamName::NewOrleansSaints,
				abbreviation: TeamAbbreviation::NO,
			}),
			Ok(TeamId::NYG) => Ok(Self {
				id,
				name: TeamName::NewYorkGiants,
				abbreviation: TeamAbbreviation::NYG,
			}),
			Ok(TeamId::NYJ) => Ok(Self {
				id,
				name: TeamName::NewYorkJets,
				abbreviation: TeamAbbreviation::NYJ,
			}),
			Ok(TeamId::PHI) => Ok(Self {
				id,
				name: TeamName::PhiladelphiaEagles,
				abbreviation: TeamAbbreviation::PHI,
			}),
			Ok(TeamId::PIT) => Ok(Self {
				id,
				name: TeamName::PittsburghSteelers,
				abbreviation: TeamAbbreviation::PIT,
			}),
			Ok(TeamId::LAC) => Ok(Self {
				id,
				name: TeamName::LosAngelesChargers,
				abbreviation: TeamAbbreviation::LAC,
			}),
			Ok(TeamId::SF) => Ok(Self {
				id,
				name: TeamName::SanFranciscoFortyNiners,
				abbreviation: TeamAbbreviation::SF,
			}),
			Ok(TeamId::SEA) => Ok(Self {
				id,
				name: TeamName::SeattleSeahawks,
				abbreviation: TeamAbbreviation::SEA,
			}),
			Ok(TeamId::LV) => Ok(Self {
				id,
				name: TeamName::LasVegasRaiders,
				abbreviation: TeamAbbreviation::LV,
			}),
			Ok(TeamId::LAR) => Ok(Self {
				id,
				name: TeamName::LosAngelesRams,
				abbreviation: TeamAbbreviation::LAR,
			}),
			Ok(TeamId::TB) => Ok(Self {
				id,
				name: TeamName::TampaBayBuccaneers,
				abbreviation: TeamAbbreviation::TB,
			}),
			Ok(TeamId::TEN) => Ok(Self {
				id,
				name: TeamName::TennesseeTitans,
				abbreviation: TeamAbbreviation::TEN,
			}),
			Ok(TeamId::WAS) => Ok(Self {
				id,
				name: TeamName::WashingtonCommanders,
				abbreviation: TeamAbbreviation::WAS,
			}),
			_ => Err(Error::NestError(NestError::unprocessable_entity(vec![("teams", "Invalid Team ID")]))),
		}
	}
}
