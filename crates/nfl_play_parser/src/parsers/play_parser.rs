
fn parse_play(text: &str) -> Result<Play, String> {
    let parts: Vec<&str> = text.split(") ").collect();
    if parts.len() < 2 {
        return Err("Invalid play format".to_string());
    }

    let clock_parts: Vec<&str> = parts[0].split(" - ").collect();
    if clock_parts.len() != 2 {
        return Err("Invalid game clock format".to_string());
    }

    let game_clock = GameClock::from_str(clock_parts[0])?;
    let mut game_clock = game_clock.clone();
    game_clock.quarter = match clock_parts[1] {
        "1st" => 1,
        "2nd" => 2,
        "3rd" => 3,
        "4th" => 4,
        "OT" => 5,
        _ => return Err("Invalid quarter".to_string()),
    };

    let description = parts[1..].join(") ");
    let play_type = determine_play_type(&description);
    let yards = extract_yards(&description);
    let players_involved = extract_players(&description);

    Ok(Play {
        game_clock,
        play_type,
        description,
        yards,
        players_involved,
    })
}

fn extract_yards(description: &str) -> i32 {
    let words: Vec<&str> = description.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        if word == &"for" && i + 1 < words.len() {
            if let Ok(yards) = words[i + 1].parse::<i32>() {
                return yards;
            }
        }
    }
    0 // Default to 0 if no yardage found
}

fn extract_players(description: &str) -> Vec<Player> {
    let mut players = Vec::new();
    let words: Vec<&str> = description.split_whitespace().collect();
    for word in words {
        if word.chars().next().map_or(false, |c| c.is_uppercase()) 
           && word.chars().nth(1).map_or(false, |c| c == '.') {
            players.push(Player {
                name: word.to_string(),
                team: String::new(), // We'd need more context to determine the team
            });
        }
    }
    players
}

fn parse_play_by_play(text: &str) -> Result<Vec<(DownAndDistance, Play)>, String> {
    let mut plays = Vec::new();
    let lines: Vec<&str> = text.lines().collect();

    let mut i = 0;
    while i < lines.len() {
        if lines[i].contains("&") {
            let down_and_distance = DownAndDistance::from_str(lines[i])?;
            if i + 1 < lines.len() {
                let play = parse_play(lines[i + 1])?;
                plays.push((down_and_distance, play));
                i += 2;
            } else {
                return Err("Unexpected end of input".to_string());
            }
        } else {
            i += 1;
        }
    }

    Ok(plays)
}
