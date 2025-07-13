use crate::lexer::{Lexer, Token, TokenType};

pub struct Redactor {
	// Future: could add configuration here
}

impl Redactor {
	pub fn new() -> Self {
		Self {}
	}

	pub fn redact_line(&self, line: &str) -> String {
		let mut lexer = Lexer::new(line.to_string());
		let tokens = lexer.tokenize();

		tokens.into_iter().map(|token| self.redact_token(token)).collect()
	}

	fn redact_token(&self, token: Token) -> String {
		match token.token_type {
			TokenType::IpAddress => "[REDACTED_IP]".to_string(),
			TokenType::Path => "[REDACTED_PATH]".to_string(),
			TokenType::Email => "[REDACTED_EMAIL]".to_string(),
			_ => token.value,
		}
	}
}
