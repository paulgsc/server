use crate::error::TeamAbbreviationError;

#[derive(Debug, Clone, PartialEq)]
pub enum TeamAbbreviation {
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
    // Add more teams as needed...
}

impl std::str::FromStr for TeamAbbreviation {
    type Err = TeamAbbreviationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ATL" => Ok(TeamAbbreviation::ATL),
            "BAL" => Ok(TeamAbbreviation::BAL),
            "BUF" => Ok(TeamAbbreviation::BUF),
            "CAR" => Ok(TeamAbbreviation::CAR),
            "CHI" => Ok(TeamAbbreviation::CHI),
            "CIN" => Ok(TeamAbbreviation::CIN),
            "CLE" => Ok(TeamAbbreviation::CLE),
            "DAL" => Ok(TeamAbbreviation::DAL),
            "DEN" => Ok(TeamAbbreviation::DEN),
            "DET" => Ok(TeamAbbreviation::DET),
            "GB" => Ok(TeamAbbreviation::GB),
            "HOU" => Ok(TeamAbbreviation::HOU),
            "IND" => Ok(TeamAbbreviation::IND),
            "JAC" => Ok(TeamAbbreviation::JAC),
            "KC" => Ok(TeamAbbreviation::KC),
            "LV" => Ok(TeamAbbreviation::LV),
            "LAC" => Ok(TeamAbbreviation::LAC),
            "LAR" => Ok(TeamAbbreviation::LAR),
            "MIA" => Ok(TeamAbbreviation::MIA),
            "MIN" => Ok(TeamAbbreviation::MIN),
            "NE" => Ok(TeamAbbreviation::NE),
            "NO" => Ok(TeamAbbreviation::NO),
            "NYG" => Ok(TeamAbbreviation::NYG),
            "NYJ" => Ok(TeamAbbreviation::NYJ),
            "PHI" => Ok(TeamAbbreviation::PHI),
            "PIT" => Ok(TeamAbbreviation::PIT),
            "SEA" => Ok(TeamAbbreviation::SEA),
            "SF" => Ok(TeamAbbreviation::SF),
            "TB" => Ok(TeamAbbreviation::TB),
            "TEN" => Ok(TeamAbbreviation::TEN),
            "WAS" => Ok(TeamAbbreviation::WAS),
            _ => Err(TeamAbbreviationError::InvalidTeamAbbreviation("Invalid Team Abbreviation Found".to_string()))
        }
    }
}

