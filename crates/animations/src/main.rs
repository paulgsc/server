use crossterm::{
	cursor,
	terminal::{self, Clear, ClearType},
	ExecutableCommand, QueueableCommand,
};
use std::{
	io::{self, Write},
	time::Duration,
};

struct Snake {
	body: String,
	length: usize,
}

impl Snake {
	fn new(length: usize) -> Self {
		Snake {
			body: format!(">{}>", "=".repeat(length.saturating_sub(2))),
			length,
		}
	}
}

struct Food {
	text: String,
}

impl Food {
	fn new(text: &str) -> Self {
		Food { text: format!("[{}]", text) }
	}

	fn eat(&mut self) -> bool {
		if self.text.len() <= 2 {
			// Just brackets left
			return false;
		}
		self.text = format!("[{}]", &self.text[1..self.text.len() - 2]);
		true
	}
}

fn animate(text: &str, snake_length: usize, speed_ms: u64) -> io::Result<()> {
	let mut stdout = io::stdout();
	let (cols, _rows) = terminal::size()?;

	terminal::enable_raw_mode()?;
	stdout.execute(cursor::Hide)?;

	let snake = Snake::new(snake_length);
	let mut food = Food::new(text);
	let mut position = 0;
	let mut eating = false;

	loop {
		stdout.queue(Clear(ClearType::All))?.queue(cursor::MoveTo(0, 0))?;

		if !eating {
			// Snake approaching food
			let spaces = " ".repeat(position);
			let gap = " ".repeat(5);
			writeln!(stdout, "{}{}{}{}", spaces, snake.body, gap, food.text)?;
			position += 1;

			if position >= cols as usize - snake.length - food.text.len() - 5 {
				eating = true;
			}
		} else {
			// Eating animation
			let spaces = " ".repeat((cols as usize) - snake.length - food.text.len() - 5);
			writeln!(stdout, "{}{}{}", spaces, snake.body, food.text)?;

			if !food.eat() {
				break;
			}
		}

		stdout.flush()?;
		std::thread::sleep(Duration::from_millis(speed_ms));
	}

	stdout.execute(cursor::Show)?;
	terminal::disable_raw_mode()?;
	println!("\nNom nom nom... done!");

	Ok(())
}

fn main() -> io::Result<()> {
	animate("Hello World", 10, 100)
}
