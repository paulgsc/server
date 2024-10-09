use std::str::FromStr;
use crate::schema::teams::TeamAbbreviation;

#[derive(Debug, Clone, PartialEq)]
pub enum Down {
    First,
    Second,
    Third,
    Fourth,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Distance(u8); // Wrapping u8 for distance

#[derive(Debug, Clone)]
pub struct DownAndDistance {
    down: Down,
    distance: Distance,
    yard_line: u8,
    side_of_ball: TeamAbbreviation,
}

impl FromStr for DownAndDistance {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() != 6 {
            return Err("Invalid down and distance format".to_string());
        }

        // Parse down
        let down = match parts[0] {
            "1st" => Down::First,
            "2nd" => Down::Second,
            "3rd" => Down::Third,
            "4th" => Down::Fourth,
            _ => return Err("Invalid down".to_string()),
        };

        // Parse distance
        let distance_value = parts[2].parse::<u8>().map_err(|_| "Invalid distance")?;
        let distance = Distance(distance_value);

        // Determine the yard line and possession
        let yard_line = parts[5].parse::<u8>().map_err(|_| "Invalid yard line")?;
        let side_of_ball = TeamAbbreviation::from_str(parts[4]).map_err(|_| "Invalid team abbreviation")?;

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

    #[test]
    fn test_valid_down_and_distance_parsing() {
        let input = "1st & 10 at ATL 30";

        let result = DownAndDistance::from_str(input);

        assert!(result.is_ok()); // Ensure the parsing is successful

        let down_and_distance = result.unwrap();
        assert_eq!(down_and_distance.down, Down::First); // Check the down is 1
        assert_eq!(down_and_distance.distance, Distance(10)); // Check the distance is 10
        assert_eq!(down_and_distance.yard_line, 30); // Check the yard line is 30

        // Check the possession details
        assert_eq!(down_and_distance.side_of_ball, TeamAbbreviation::ATL); // Adjust based on your logic
    }

    #[test]
    fn test_invalid_down_and_distance_parsing_invalid_format() {
        let input = "Invalid format";

        let result = DownAndDistance::from_str(input);
        assert!(result.is_err()); // Ensure it returns an error
        assert_eq!(result.err().unwrap(), "Invalid down and distance format".to_string());
    }

    #[test]
    fn test_invalid_down_and_distance_parsing_invalid_distance() {
        let input = "1st & XX at ATL 30";

        let result = DownAndDistance::from_str(input);
        assert!(result.is_err()); // Ensure it returns an error
        assert_eq!(result.err().unwrap(), "Invalid distance".to_string());
    }

    #[test]
    fn test_invalid_down_and_distance_parsing_invalid_yard_line() {
        let input = "1st & 10 at ATL XX";

        let result = DownAndDistance::from_str(input);
        assert!(result.is_err()); // Ensure it returns an error
        assert_eq!(result.err().unwrap(), "Invalid yard line".to_string());
    }

    #[test]
    fn test_invalid_down_parsing() {
        let input = "Invalid down & 10 at ATL 30";

        let result = DownAndDistance::from_str(input);
        assert!(result.is_err()); // Ensure it returns an error
        assert_eq!(result.err().unwrap(), "Invalid down".to_string());
    }

    #[test]
    fn test_invalid_team_abbreviation() {
        let input = "1st & 10 at XYZ 30"; // XYZ is an invalid team abbreviation

        let result = DownAndDistance::from_str(input);
        assert!(result.is_err()); // Ensure it returns an error
        assert_eq!(result.err().unwrap(), "Invalid team abbreviation".to_string());
    }
}

