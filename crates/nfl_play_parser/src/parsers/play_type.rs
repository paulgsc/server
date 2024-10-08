#[derive(Debug)]
enum PlayParseError {
    UnknownPlayType(String),
}

// Function that determines the play type based on the description
fn determine_play_type(description: &str) -> Result<PlayType, PlayParseError> {
    match description {
        s if s.contains("kicks") => Ok(PlayType::Kickoff),
        s if s.contains("pass") => Ok(PlayType::Pass),
        s if s.contains("run") || s.contains("scrambles") => Ok(PlayType::Run),
        s if s.contains("punts") => Ok(PlayType::Punt),
        s if s.contains("field goal") => Ok(PlayType::FieldGoal),
        s if s.contains("extra point") => Ok(PlayType::ExtraPoint),
        s if s.contains("PENALTY") => Ok(PlayType::Penalty),
        s if s.contains("Timeout") => Ok(PlayType::Timeout),
        s if s.contains("Two-Point") => Ok(PlayType::TwoPointConversion),
        _ => Err(PlayParseError::UnknownPlayType(description.to_string())), // Return an error for unknown play types
    }

}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determine_play_type() {
        assert!(matches!(determine_play_type("T.Gill kicks 65 yards"), PlayType::Kickoff));
        assert!(matches!(determine_play_type("K.Cousins pass deep right"), PlayType::Pass));
        assert!(matches!(determine_play_type("B.Robinson right tackle"), PlayType::Run));
        assert!(matches!(determine_play_type("M.Dickson punts 52 yards"), PlayType::Punt));
        assert!(matches!(determine_play_type("J.Tucker 47 yard field goal is GOOD"), PlayType::FieldGoal));
        assert!(matches!(determine_play_type("J.Tucker extra point is GOOD"), PlayType::ExtraPoint));
        assert!(matches!(determine_play_type("PENALTY on TB"), PlayType::Penalty));
        assert!(matches!(determine_play_type("Timeout #1 by TB"), PlayType::Timeout));
        assert!(matches!(determine_play_type("Two-Point Conversion Attempt"), PlayType::TwoPointConversion));
        assert!(matches!(determine_play_type("Unknown play description"), PlayType::Run));
    }

}
