use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::str::Chars;

/// Represents a token in the TSX source code.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Token {
	pub token_type: TokenType,
	pub literal: String,
	pub line: usize,
	pub column: usize,
}

/// Enum representing the type of a token.
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum TokenType {
	// Single-character tokens.
	BraceL,    // {
	BraceR,    // }
	BracketL,  // [
	BracketR,  // ]
	ParenL,    // (
	ParenR,    // )
	Comma,     // ,
	Colon,     // :
	Dot,       // .
	Minus,     // -
	Plus,      // +
	Semicolon, // ;
	Slash,     // /
	Star,      // *
	Tilde,     // ~
	Bang,      // !
	Question,  // ?
	Eq,        // =
	Lt,        // <
	Gt,        // >

	//複合
	EqEq,     // ==
	NotEq,    // !=
	LtEq,     // <=
	GtEq,     // >=
	PlusEq,   // +=
	MinusEq,  // -=
	StarEq,   // *=
	SlashEq,  // /=
	EqEqEq,   // ===
	NotEqEq,  // !==
	Amp,      // &
	AmpAmp,   // &&
	Pipe,     // |
	PipePipe, // ||

	// Keywords.
	KeywordAsync,
	KeywordAwait,
	KeywordBreak,
	KeywordCase,
	KeywordCatch,
	KeywordClass,
	KeywordConst,
	KeywordContinue,
	KeywordDebugger,
	KeywordDefault,
	KeywordDelete,
	KeywordDo,
	KeywordElse,
	KeywordEnum,
	KeywordExport,
	KeywordExtends,
	KeywordFalse,
	KeywordFinally,
	KeywordFor,
	KeywordFrom,
	KeywordFunction,
	KeywordIf,
	KeywordImport,
	KeywordIn,
	KeywordInstanceOf,
	KeywordNew,
	KeywordNull,
	KeywordReturn,
	KeywordSet,
	KeywordStatic,
	KeywordSuper,
	KeywordSwitch,
	KeywordThis,
	KeywordThrow,
	KeywordTrue,
	KeywordTry,
	KeywordTypeOf,
	KeywordVar,
	KeywordVoid,
	KeywordWhile,
	KeywordWith,
	KeywordYield,
	KeywordInterface, //TS
	KeywordLet,       //TS
	KeywordNumber,    //TS
	KeywordString,    //TS
	KeywordBoolean,   //TS
	KeywordAny,       //TS
	KeywordVoidType,  //TS void keyword
	KeywordUndefined, //TS undefined
	KeywordNever,     //TS never
	KeywordUnknown,   //TS unknown
	KeywordBigInt,    //TS bigint
	KeywordSymbol,    //TS Symbol
	KeywordPublic,    //TS public
	KeywordPrivate,   //TS private
	KeywordProtected, //TS protected
	KeywordAbstract,  //TS abstract
	KeywordReadonly,  //TS readonly
	KeywordDeclare,   //TS declare
	KeywordType,      //TS type
	KeywordKeyOf,     //TS keyof
	KeywordAsserts,   //TS asserts
	KeywordIs,        //TS is
	KeywordAs,        //TS as
	KeywordNameSpace, //TS namespace
	KeywordModule,    //TS module
	KeywordRequire,   //TS require

	// JSX specific tokens
	JSXText,
	JSXIdentifier,
	JSXAttributeName,
	JSXAttributeStringValue,
	JSXAttributeExpressionValue,
	JSXOpeningElementEnd,   // >,  />
	JSXClosingElementStart, // </
	JSXFragmentOpening,     // <>, <Something>
	JSXFragmentClosing,     // </>
	JSXSpread,              // {...
	// Literals.
	Identifier,
	String,
	Number,
	BigInt,
	RegEx,

	// End of file.
	EOF,

	// Invalid Type
	Illegal,
}

// Lookup table for single character tokens
static SINGLE_CHAR_TOKENS: Lazy<HashMap<char, TokenType>> = Lazy::new(|| {
	let mut m = HashMap::new();
	m.insert('{', TokenType::BraceL);
	m.insert('}', TokenType::BraceR);
	m.insert('[', TokenType::BracketL);
	m.insert(']', TokenType::BracketR);
	m.insert('(', TokenType::ParenL);
	m.insert(')', TokenType::ParenR);
	m.insert(',', TokenType::Comma);
	m.insert(':', TokenType::Colon);
	m.insert('.', TokenType::Dot);
	m.insert('+', TokenType::Plus);
	m.insert('-', TokenType::Minus);
	m.insert(';', TokenType::Semicolon);
	m.insert('*', TokenType::Star);
	m.insert('~', TokenType::Tilde);
	m.insert('!', TokenType::Bang);
	m.insert('?', TokenType::Question);
	m.insert('=', TokenType::Eq);
	m.insert('<', TokenType::Lt);
	m.insert('>', TokenType::Gt);
	m.insert('&', TokenType::Amp);
	m.insert('|', TokenType::Pipe);
	m
});

// Lookup table for two-character tokens
static TWO_CHAR_TOKENS: Lazy<HashMap<(char, char), TokenType>> = Lazy::new(|| {
	let mut m = HashMap::new();
	m.insert(('=', '='), TokenType::EqEq);
	m.insert(('!', '='), TokenType::NotEq);
	m.insert(('<', '='), TokenType::LtEq);
	m.insert(('>', '='), TokenType::GtEq);
	m.insert(('+', '='), TokenType::PlusEq);
	m.insert(('-', '='), TokenType::MinusEq);
	m.insert(('*', '='), TokenType::StarEq);
	m.insert(('/', '='), TokenType::SlashEq);
	m.insert(('&', '&'), TokenType::AmpAmp);
	m.insert(('|', '|'), TokenType::PipePipe);
	m.insert(('<', '>'), TokenType::JSXFragmentOpening);
	m.insert(('/', '>'), TokenType::JSXOpeningElementEnd);
	m.insert(('<', '/'), TokenType::JSXClosingElementStart);
	m
});

// Lookup table for three-character tokens
static THREE_CHAR_TOKENS: Lazy<HashMap<(char, char, char), TokenType>> = Lazy::new(|| {
	let mut m = HashMap::new();
	m.insert(('=', '=', '='), TokenType::EqEqEq);
	m.insert(('!', '=', '='), TokenType::NotEqEq);
	m
});

// Lookup table for keywords
static KEYWORDS: Lazy<HashMap<&'static str, TokenType>> = Lazy::new(|| {
	let mut m = HashMap::new();
	m.insert("async", TokenType::KeywordAsync);
	m.insert("await", TokenType::KeywordAwait);
	m.insert("break", TokenType::KeywordBreak);
	m.insert("case", TokenType::KeywordCase);
	m.insert("catch", TokenType::KeywordCatch);
	m.insert("class", TokenType::KeywordClass);
	m.insert("const", TokenType::KeywordConst);
	m.insert("continue", TokenType::KeywordContinue);
	m.insert("debugger", TokenType::KeywordDebugger);
	m.insert("default", TokenType::KeywordDefault);
	m.insert("delete", TokenType::KeywordDelete);
	m.insert("do", TokenType::KeywordDo);
	m.insert("else", TokenType::KeywordElse);
	m.insert("enum", TokenType::KeywordEnum);
	m.insert("export", TokenType::KeywordExport);
	m.insert("extends", TokenType::KeywordExtends);
	m.insert("false", TokenType::KeywordFalse);
	m.insert("finally", TokenType::KeywordFinally);
	m.insert("for", TokenType::KeywordFor);
	m.insert("from", TokenType::KeywordFrom);
	m.insert("function", TokenType::KeywordFunction);
	m.insert("if", TokenType::KeywordIf);
	m.insert("import", TokenType::KeywordImport);
	m.insert("in", TokenType::KeywordIn);
	m.insert("instanceof", TokenType::KeywordInstanceOf);
	m.insert("new", TokenType::KeywordNew);
	m.insert("null", TokenType::KeywordNull);
	m.insert("return", TokenType::KeywordReturn);
	m.insert("set", TokenType::KeywordSet);
	m.insert("static", TokenType::KeywordStatic);
	m.insert("super", TokenType::KeywordSuper);
	m.insert("switch", TokenType::KeywordSwitch);
	m.insert("this", TokenType::KeywordThis);
	m.insert("throw", TokenType::KeywordThrow);
	m.insert("true", TokenType::KeywordTrue);
	m.insert("try", TokenType::KeywordTry);
	m.insert("typeof", TokenType::KeywordTypeOf);
	m.insert("var", TokenType::KeywordVar);
	m.insert("void", TokenType::KeywordVoid);
	m.insert("while", TokenType::KeywordWhile);
	m.insert("with", TokenType::KeywordWith);
	m.insert("yield", TokenType::KeywordYield);
	m.insert("interface", TokenType::KeywordInterface);
	m.insert("let", TokenType::KeywordLet);
	m.insert("number", TokenType::KeywordNumber);
	m.insert("string", TokenType::KeywordString);
	m.insert("boolean", TokenType::KeywordBoolean);
	m.insert("any", TokenType::KeywordAny);
	m.insert("void", TokenType::KeywordVoidType);
	m.insert("undefined", TokenType::KeywordUndefined);
	m.insert("never", TokenType::KeywordNever);
	m.insert("unknown", TokenType::KeywordUnknown);
	m.insert("bigint", TokenType::KeywordBigInt);
	m.insert("symbol", TokenType::KeywordSymbol);
	m.insert("public", TokenType::KeywordPublic);
	m.insert("private", TokenType::KeywordPrivate);
	m.insert("protected", TokenType::KeywordProtected);
	m.insert("abstract", TokenType::KeywordAbstract);
	m.insert("readonly", TokenType::KeywordReadonly);
	m.insert("declare", TokenType::KeywordDeclare);
	m.insert("type", TokenType::KeywordType);
	m.insert("keyof", TokenType::KeywordKeyOf);
	m.insert("asserts", TokenType::KeywordAsserts);
	m.insert("is", TokenType::KeywordIs);
	m.insert("as", TokenType::KeywordAs);
	m.insert("namespace", TokenType::KeywordNameSpace);
	m.insert("module", TokenType::KeywordModule);
	m.insert("require", TokenType::KeywordRequire);
	m
});

/// Represents the lexer, which tokenizes the input string.
#[derive(Clone)]
pub struct Lexer<'a> {
	pub input: &'a str,
	chars: Chars<'a>,
	current_char: Option<char>,
	line: usize,
	column: usize,
	read_position: usize,
}

impl<'a> Lexer<'a> {
	/// Creates a new lexer with the given input string.
	#[must_use]
	pub fn new(input: &'a str) -> Self {
		let mut lexer = Lexer {
			input,
			chars: input.chars(),
			current_char: None,
			line: 1,
			column: 0,
			read_position: 0,
		};
		lexer.read_char(); // Initialize the first character.
		lexer
	}

	/// Reads the next character from the input.
	fn read_char(&mut self) {
		self.current_char = self.chars.next();
		self.read_position += 1;
		self.column += 1;
		if self.current_char == Some('\n') {
			self.line += 1;
			self.column = 0;
		}
	}

	/// Peeks at the next character without consuming it.
	fn peek_char(&self) -> Option<char> {
		self.chars.clone().next()
	}

	/// Peeks at the second next character without consuming it.
	fn peek_second_char(&self) -> Option<char> {
		let mut chars_clone = self.chars.clone();
		chars_clone.next(); // Skip one
		chars_clone.next() // Get the second
	}

	/// Collects characters until a certain condition is met
	fn read_while<F>(&mut self, condition: F) -> String
	where
		F: Fn(char) -> bool,
	{
		let mut result = String::new();
		while let Some(ch) = self.current_char {
			if condition(ch) {
				result.push(ch);
				self.read_char();
			} else {
				break;
			}
		}
		result
	}

	/// Skips whitespace characters.
	fn skip_whitespace(&mut self) {
		self.read_while(char::is_whitespace);
	}

	/// Reads an identifier or keyword.
	fn read_identifier(&mut self) -> String {
		self.read_while(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
	}

	/// Reads a number, including decimals and exponents.
	fn read_number(&mut self) -> String {
		let mut number_string = String::new();
		while let Some(ch) = self.current_char {
			if ch.is_ascii_digit() || ch == '.' || ch == 'e' || ch == 'E' || ch == '-' || ch == '+' {
				number_string.push(ch);
				self.read_char();
			} else {
				break;
			}
		}
		number_string
	}

	/// Reads a string literal.
	fn read_string(&mut self, quote: char) -> String {
		let mut string_literal = String::new();
		self.read_char(); // Consume the opening quote.

		while let Some(ch) = self.current_char {
			if ch == quote {
				self.read_char(); // Consume the closing quote.
				break;
			} else if ch == '\\' {
				self.read_char(); // Consume the backslash
				if let Some(next_ch) = self.current_char {
					string_literal.push(next_ch);
					self.read_char();
				}
			} else {
				string_literal.push(ch);
				self.read_char();
			}
		}
		string_literal
	}

	fn read_regex(&mut self) -> String {
		let mut regex_literal = String::new();
		self.read_char(); // Consume the initial '/'.

		while let Some(ch) = self.current_char {
			match ch {
				'/' => {
					self.read_char(); // Consume the closing '/'.
					break;
				}
				'\\' => {
					regex_literal.push(ch);
					self.read_char();
					if let Some(next_ch) = self.current_char {
						regex_literal.push(next_ch);
						self.read_char();
					}
				}
				_ => {
					regex_literal.push(ch);
					self.read_char();
				}
			}
		}
		regex_literal
	}

	fn skip_comment(&mut self) {
		while let Some(ch) = self.current_char {
			if ch == '*' && self.peek_char() == Some('/') {
				self.read_char(); // Consume '*'
				self.read_char(); // Consume '/'
				break;
			}
			self.read_char();
		}
	}

	fn skip_line_comment(&mut self) {
		while let Some(ch) = self.current_char {
			if ch == '\n' {
				break;
			}
			self.read_char();
		}
	}

	/// Gets the next token from the input.
	pub fn next_token(&mut self) -> Token {
		self.skip_whitespace();

		let line = self.line;
		let column = self.column;

		if self.current_char.is_none() {
			return Token::new(TokenType::EOF, String::new(), self.line, self.column);
		}

		let ch = self.current_char.unwrap();
		if ch == '"' || ch == '\'' || ch == '`' {
			let string_literal = self.read_string(ch);
			return Token::new(TokenType::String, string_literal, line, column);
		}

		if ch.is_ascii_digit() {
			let number_literal = self.read_number();
			if number_literal.ends_with('n') {
				return Token::new(TokenType::BigInt, number_literal, line, column);
			}
			return Token::new(TokenType::Number, number_literal, line, column);
		}
		if ch.is_alphabetic() || ch == '_' || ch == '$' {
			let identifier = self.read_identifier();
			if let Some(&token_type) = KEYWORDS.get(identifier.as_str()) {
				return Token::new(token_type, identifier, line, column);
			}
			return Token::new(TokenType::Identifier, identifier, line, column);
		}

		// Special case for comments and regex
		if ch == '/' {
			let next_char = self.peek_char();
			match next_char {
				Some('/') => {
					self.read_char();
					self.read_char();
					self.skip_line_comment();
					return self.next_token();
				}
				Some('*') => {
					self.read_char();
					self.read_char();
					self.skip_comment();
					return self.next_token();
				}
				_ => {}
			}
		}

		// Check for three-character tokens
		if let Some(next_ch) = self.peek_char() {
			if let Some(next_next_ch) = self.peek_second_char() {
				if let Some(&token_type) = THREE_CHAR_TOKENS.get(&(ch, next_ch, next_next_ch)) {
					let literal = format!("{ch}{next_ch}{next_next_ch}");
					self.read_char();
					self.read_char();
					self.read_char();
					return Token::new(token_type, literal, line, column);
				}
			}
		}

		// Check for two-character tokens
		if let Some(next_ch) = self.peek_char() {
			if let Some(&token_type) = TWO_CHAR_TOKENS.get(&(ch, next_ch)) {
				let literal = format!("{ch}{next_ch}");
				self.read_char();
				self.read_char();
				return Token::new(token_type, literal, line, column);
			}
		}

		// Handle regex as a special case
		if ch == '/' && !matches!(self.peek_char(), Some('=') | Some('/') | Some('*') | Some('>')) {
			let regex_literal = self.read_regex();
			return Token::new(TokenType::RegEx, regex_literal, line, column);
		}

		// Check for single-character tokens
		if let Some(&token_type) = SINGLE_CHAR_TOKENS.get(&ch) {
			let literal = ch.to_string();
			self.read_char();
			return Token::new(token_type, literal, line, column);
		}

		// If we get here, it's an unknown character
		let literal = ch.to_string();
		self.read_char();
		Token::new(TokenType::Illegal, literal, line, column)
	}
}

impl Iterator for Lexer<'_> {
	type Item = Token;

	fn next(&mut self) -> Option<Self::Item> {
		let token = self.next_token();
		if token.token_type == TokenType::EOF {
			None
		} else {
			Some(token)
		}
	}
}

impl Token {
	/// Creates a new token.
	#[must_use]
	pub const fn new(token_type: TokenType, literal: String, line: usize, column: usize) -> Self {
		Self {
			token_type,
			literal,
			line,
			column,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_basic_token_types() {
		let input = "<div>hello</div>";
		let lexer = Lexer::new(input);
		let tokens: Vec<TokenType> = lexer.map(|t| t.token_type).collect();
		assert_eq!(
			tokens,
			vec![
				TokenType::Lt,
				TokenType::Identifier,
				TokenType::Gt,
				TokenType::Identifier,
				TokenType::JSXClosingElementStart,
				TokenType::Identifier,
				TokenType::Gt
			]
		);
	}

	#[test]
	fn test_basic_literals() {
		let input = "<div>hello, world</div>";
		let lexer = Lexer::new(input);
		let tokens: Vec<String> = lexer.map(|t| t.literal).collect();
		assert_eq!(
			tokens,
			vec![
				"<".to_string(),
				"div".to_string(),
				">".to_string(),
				"hello".to_string(),
				",".to_string(),
				"world".to_string(),
				"</".to_string(),
				"div".to_string(),
				">".to_string(),
			]
		);
	}

	#[test]
	fn test_self_closing_tag() {
		let input = "<input type=\"text\" disabled />";
		let lexer = Lexer::new(input);
		let tokens: Vec<TokenType> = lexer.map(|t| t.token_type).collect();
		assert_eq!(
			tokens,
			vec![
				TokenType::Lt,
				TokenType::Identifier,
				TokenType::KeywordType,
				TokenType::Eq,
				TokenType::String,
				TokenType::Identifier,
				TokenType::JSXOpeningElementEnd
			]
		);
	}
	#[test]
	fn test_lexer_basic() {
		let input = "let x = 42;";
		let lexer = Lexer::new(input);
		let tokens: Vec<TokenType> = lexer.map(|t| t.token_type).collect();
		assert_eq!(
			tokens,
			vec![TokenType::KeywordLet, TokenType::Identifier, TokenType::Eq, TokenType::Number, TokenType::Semicolon,]
		);
	}
}
