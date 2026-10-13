//! The league's fixed shape: 32 teams, two conferences of four divisions.
//!
//! Codes are the ones the client contract uses (`LAR`, `WAS`, `LV`); a feed
//! that spells them differently is mapped at its adapter, never here.

use serde::{Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Conference {
	Afc,
	Nfc,
}

impl Conference {
	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::Afc => "AFC",
			Self::Nfc => "NFC",
		}
	}
}

/// One of the eight divisions, as an index `0..8`: AFC East, North, South,
/// West, then the NFC's four in the same order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Division(u8);

impl Division {
	#[must_use]
	pub const fn conference(self) -> Conference {
		if self.0 < 4 {
			Conference::Afc
		} else {
			Conference::Nfc
		}
	}

	#[must_use]
	pub const fn index(self) -> usize {
		self.0 as usize
	}
}

macro_rules! teams {
	($($variant:ident = $code:literal, $name:literal, $division:literal;)*) => {
		/// A team, by the code the client contract shows.
		#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
		pub enum Team {
			$($variant,)*
		}

		impl Team {
			/// Every team, in division order (four per division).
			pub const ALL: [Self; 32] = [$(Self::$variant,)*];

			#[must_use]
			pub const fn code(self) -> &'static str {
				match self {
					$(Self::$variant => $code,)*
				}
			}

			/// The short name a fan says: "Niners", "Commanders".
			#[must_use]
			pub const fn name(self) -> &'static str {
				match self {
					$(Self::$variant => $name,)*
				}
			}

			#[must_use]
			pub const fn division(self) -> Division {
				match self {
					$(Self::$variant => Division($division),)*
				}
			}

			/// Parses a contract code. Total: an unknown code is `None`, never a panic.
			#[must_use]
			pub fn from_code(code: &str) -> Option<Self> {
				match code {
					$($code => Some(Self::$variant),)*
					_ => None,
				}
			}
		}
	};
}

teams! {
	Buf = "BUF", "Bills", 0;
	Mia = "MIA", "Dolphins", 0;
	Ne = "NE", "Patriots", 0;
	Nyj = "NYJ", "Jets", 0;
	Bal = "BAL", "Ravens", 1;
	Pit = "PIT", "Steelers", 1;
	Cin = "CIN", "Bengals", 1;
	Cle = "CLE", "Browns", 1;
	Hou = "HOU", "Texans", 2;
	Ind = "IND", "Colts", 2;
	Jax = "JAX", "Jaguars", 2;
	Ten = "TEN", "Titans", 2;
	Kc = "KC", "Chiefs", 3;
	Lac = "LAC", "Chargers", 3;
	Den = "DEN", "Broncos", 3;
	Lv = "LV", "Raiders", 3;
	Phi = "PHI", "Eagles", 4;
	Dal = "DAL", "Cowboys", 4;
	Was = "WAS", "Commanders", 4;
	Nyg = "NYG", "Giants", 4;
	Det = "DET", "Lions", 5;
	Gb = "GB", "Packers", 5;
	Min = "MIN", "Vikings", 5;
	Chi = "CHI", "Bears", 5;
	Tb = "TB", "Buccaneers", 6;
	Atl = "ATL", "Falcons", 6;
	No = "NO", "Saints", 6;
	Car = "CAR", "Panthers", 6;
	Sf = "SF", "Niners", 7;
	Lar = "LAR", "Rams", 7;
	Sea = "SEA", "Seahawks", 7;
	Ari = "ARI", "Cardinals", 7;
}

impl Team {
	/// A dense index `0..32`, for tables and bitsets.
	#[must_use]
	pub const fn index(self) -> usize {
		self as usize
	}

	#[must_use]
	pub const fn conference(self) -> Conference {
		self.division().conference()
	}

	/// The other three teams in this team's division.
	pub fn division_rivals(self) -> impl Iterator<Item = Self> {
		let division = self.division();
		Self::ALL.into_iter().filter(move |t| *t != self && t.division() == division)
	}

	/// The sixteen teams of a conference, in division order.
	pub fn of_conference(conference: Conference) -> impl Iterator<Item = Self> {
		Self::ALL.into_iter().filter(move |t| t.conference() == conference)
	}
}

impl Serialize for Team {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(self.code())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_division_has_four_teams_and_codes_round_trip() {
		for d in 0..8 {
			assert_eq!(Team::ALL.iter().filter(|t| t.division().index() == d).count(), 4);
		}
		for (i, t) in Team::ALL.into_iter().enumerate() {
			assert_eq!(t.index(), i);
			assert_eq!(Team::from_code(t.code()), Some(t));
		}
		assert_eq!(Team::of_conference(Conference::Nfc).count(), 16);
		assert_eq!(Team::Sf.division_rivals().collect::<Vec<_>>(), [Team::Lar, Team::Sea, Team::Ari]);
	}
}
