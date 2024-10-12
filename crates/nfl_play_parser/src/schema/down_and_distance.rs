use std::str::FromStr;
use crate::schema::TeamAbbreviation;
use crate::error::DownAndDistanceError;

#[derive(Debug, Clone, PartialEq)]
pub enum Down {
    First,
    Second,
    Third,
    Fourth,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Distance(u8); // Wrapping u8 for distance

#[derive(Debug, Clone, PartialEq)]
pub struct DownAndDistance {
    down: Down,
    distance: Distance,
    yard_line: u8,
    side_of_ball: TeamAbbreviation,
}

impl FromStr for DownAndDistance {
    type Err = DownAndDistanceError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() != 6 {
            return Err(DownAndDistanceError::InvalidDownDistanceFormat);
        }

        // Parse down
        let down = match parts[0] {
            "1st" => Down::First,
            "2nd" => Down::Second,
            "3rd" => Down::Third,
            "4th" => Down::Fourth,
            _ => return Err(DownAndDistanceError::InvalidDown),
        };

        let distance_value = parts[2].parse::<u8>().map_err(|_| DownAndDistanceError::InvalidDownDistance)?;
        let distance = Distance(distance_value);

        let yard_line = parts[5].parse::<u8>().map_err(|_| DownAndDistanceError::InvalidYardLine)?;
        let side_of_ball = TeamAbbreviation::from_str(parts[4])?;

        Ok(DownAndDistance {
            down,
            distance,
            yard_line,
            side_of_ball,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use crate::error::{DownAndDistanceError, TeamAbbreviationError};



    #[test]
    fn test_valid_down_and_distance_parsing() {
        let input = "1st & 10 at ATL 30";
        let result = DownAndDistance::from_str(input);
        assert!(result.is_ok());
        let down_and_distance = result.unwrap();
        assert_eq!(down_and_distance.down, Down::First);
        assert_eq!(down_and_distance.distance, Distance(10));
        assert_eq!(down_and_distance.yard_line, 30);
        assert_eq!(down_and_distance.side_of_ball, TeamAbbreviation::ATL);
    }

    #[test]
    fn test_invalid_down_and_distance_parsing_invalid_format() {
        let input = "Invalid format";
        let result = DownAndDistance::from_str(input);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), DownAndDistanceError::InvalidDownDistanceFormat);
    }

    #[test]
    fn test_invalid_down_and_distance_parsing_invalid_distance() {
        let input = "1st & XX at ATL 30";
        let result = DownAndDistance::from_str(input);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), DownAndDistanceError::InvalidDownDistance);
    }

    #[test]
    fn test_invalid_down_and_distance_parsing_invalid_yard_line() {
        let input = "1st & 10 at ATL XX";
        let result = DownAndDistance::from_str(input);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), DownAndDistanceError::InvalidYardLine);
    }

    #[test]
    fn test_invalid_down_parsing() {
        let input = "5st & 10 at TB 18";
        let result = DownAndDistance::from_str(input);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), DownAndDistanceError::InvalidDown);
    }

    #[test]
    fn test_invalid_team_abbreviation() {
        let input = "1st & 10 at XYZ 30";
        let result = DownAndDistance::from_str(input);
        assert!(result.is_err());

        match result.unwrap_err() {
            DownAndDistanceError::TeamAbbreviationError(TeamAbbreviationError::InvalidTeamAbbreviation(_)) => {}
            _ => panic!("Expected InvalidTeamAbbreviation error"),
        }
    }
}
