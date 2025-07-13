#[derive(Debug, Clone, PartialEq)]
pub enum TokenType {
	Word,
	Number,
	Punctuation,
	Whitespace,
	Path,
	Email,
	IpAddress,
	Other,
}

#[derive(Debug, Clone)]
pub struct Token {
	pub token_type: TokenType,
	pub value: String,
	pub start: usize,
	pub end: usize,
}

pub struct Lexer {
	input: String,
	position: usize,
	current_char: Option<char>,
}

impl Lexer {
	pub fn new(input: String) -> Self {
		let current_char = input.chars().next();
		Self { input, position: 0, current_char }
	}

	pub fn tokenize(&mut self) -> Vec<Token> {
		let mut tokens = Vec::new();

		while self.current_char.is_some() {
			let start_pos = self.position;

			if let Some(ch) = self.current_char {
				match ch {
					' ' | '\t' | '\n' | '\r' => {
						let value = self.consume_whitespace();
						tokens.push(Token {
							token_type: TokenType::Whitespace,
							value,
							start: start_pos,
							end: self.position,
						});
					}
					'/' => {
						let value = self.consume_potential_path();
						let token_type = if self.looks_like_path(&value) { TokenType::Path } else { TokenType::Punctuation };
						tokens.push(Token {
							token_type,
							value,
							start: start_pos,
							end: self.position,
						});
					}
					'0'..='9' => {
						let value = self.consume_number_sequence();
						let token_type = if self.looks_like_ip(&value) { TokenType::IpAddress } else { TokenType::Number };
						tokens.push(Token {
							token_type,
							value,
							start: start_pos,
							end: self.position,
						});
					}
					'a'..='z' | 'A'..='Z' | '_' => {
						let value = self.consume_word();
						let token_type = if self.looks_like_email(&value) { TokenType::Email } else { TokenType::Word };
						tokens.push(Token {
							token_type,
							value,
							start: start_pos,
							end: self.position,
						});
					}
					_ => {
						let value = self.advance().to_string();
						tokens.push(Token {
							token_type: TokenType::Punctuation,
							value,
							start: start_pos,
							end: self.position,
						});
					}
				}
			}
		}

		tokens
	}

	fn advance(&mut self) -> char {
		let ch = self.current_char.unwrap();
		self.position += ch.len_utf8();
		self.current_char = self.input.chars().nth(self.char_position());
		ch
	}

	fn char_position(&self) -> usize {
		self.input[..self.position].chars().count()
	}

	fn consume_whitespace(&mut self) -> String {
		let mut result = String::new();
		while let Some(ch) = self.current_char {
			if ch.is_whitespace() {
				result.push(self.advance());
			} else {
				break;
			}
		}
		result
	}

	fn consume_word(&mut self) -> String {
		let mut result = String::new();
		while let Some(ch) = self.current_char {
			if ch.is_alphanumeric() || ch == '_' || ch == '.' || ch == '@' || ch == '-' || ch == '+' || ch == '%' {
				result.push(self.advance());
			} else {
				break;
			}
		}
		result
	}

	fn consume_number_sequence(&mut self) -> String {
		let mut result = String::new();
		while let Some(ch) = self.current_char {
			if ch.is_ascii_digit() || ch == '.' {
				result.push(self.advance());
			} else {
				break;
			}
		}
		result
	}

	fn consume_potential_path(&mut self) -> String {
		let mut result = String::new();
		result.push(self.advance()); // consume the initial '/'

		while let Some(ch) = self.current_char {
			if ch.is_alphanumeric() || ch == '/' || ch == '.' || ch == '_' || ch == '-' || ch == '~' {
				result.push(self.advance());
			} else {
				break;
			}
		}
		result
	}

	fn looks_like_path(&self, value: &str) -> bool {
		value.starts_with('/') && value.len() > 1 && value.chars().any(|c| c.is_alphanumeric() || c == '_' || c == '-')
	}

	fn looks_like_email(&self, value: &str) -> bool {
		value.contains('@') && value.contains('.') && value.chars().all(|c| c.is_alphanumeric() || "@._%+-".contains(c))
	}

	fn looks_like_ip(&self, value: &str) -> bool {
		let parts: Vec<&str> = value.split('.').collect();
		if parts.len() != 4 {
			return false;
		}

		parts.iter().all(|part| if let Ok(num) = part.parse::<u8>() { num <= 255 } else { false })
	}
}
