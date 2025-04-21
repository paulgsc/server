use crate::lexer::{Lexer, Token, TokenType};
use std::iter::Peekable;

/// AST node types for the TSX language
#[derive(Debug, PartialEq, Clone)]
pub enum Node {
	Program {
		body: Vec<Node>,
	},
	VariableDeclaration {
		kind: VariableKind, // let, const, var
		declarations: Vec<Node>,
	},
	VariableDeclarator {
		id: Box<Node>,
		init: Option<Box<Node>>,
	},
	Identifier {
		name: String,
	},
	JSXIdentifier {
		name: String,
	},
	Literal {
		value: LiteralValue,
	},
	ExpressionStatement {
		expression: Box<Node>,
	},
	BinaryExpression {
		operator: BinaryOperator,
		left: Box<Node>,
		right: Box<Node>,
	},
	AssignmentExpression {
		operator: AssignmentOperator,
		left: Box<Node>,
		right: Box<Node>,
	},
	BlockStatement {
		body: Vec<Node>,
	},
	IfStatement {
		test: Box<Node>,
		consequent: Box<Node>,
		alternate: Option<Box<Node>>,
	},
	FunctionDeclaration {
		id: Option<Box<Node>>,
		params: Vec<Node>,
		body: Box<Node>,
		async_: bool,
		generator: bool,
	},
	ReturnStatement {
		argument: Option<Box<Node>>,
	},
	JSXElement {
		opening_element: Box<Node>,
		children: Vec<Node>,
		closing_element: Option<Box<Node>>,
	},
	JSXOpeningElement {
		name: Box<Node>,
		attributes: Vec<Node>,
		self_closing: bool,
	},
	JSXClosingElement {
		name: Box<Node>,
	},
	JSXAttribute {
		name: String,
		value: Option<Box<Node>>,
	},
	JSXText {
		value: String,
	},
	ImportDeclaration {
		specifiers: Vec<Node>,
		source: Box<Node>,
	},
	ImportSpecifier {
		imported: Box<Node>,
		local: Box<Node>,
	},
	ExportDeclaration {
		declaration: Box<Node>,
	},
}

#[derive(Debug, PartialEq, Clone)]
pub enum VariableKind {
	Let,
	Const,
	Var,
}

#[derive(Debug, PartialEq, Clone)]
pub enum LiteralValue {
	String(String),
	Number(f64),
	Boolean(bool),
	Null,
	Undefined,
	RegExp(String),
	BigInt(String),
}

#[derive(Debug, PartialEq, Clone)]
pub enum BinaryOperator {
	Equal,            // ==
	NotEqual,         // !=
	StrictEqual,      // ===
	StrictNotEqual,   // !==
	LessThan,         // <
	LessThanEqual,    // <=
	GreaterThan,      // >
	GreaterThanEqual, // >=
	Add,              // +
	Subtract,         // -
	Multiply,         // *
	Divide,           // /
	And,              // &&
	Or,               // ||
}

#[derive(Debug, PartialEq, Clone)]
pub enum AssignmentOperator {
	Equals,       // =
	PlusEquals,   // +=
	MinusEquals,  // -=
	TimesEquals,  // *=
	DivideEquals, // /=
}

#[derive(Debug)]
pub enum ParseError {
	UnexpectedToken {
		expected: Vec<TokenType>,
		found: TokenType,
		line: usize,
		column: usize,
	},
	UnexpectedEOF {
		expected: Vec<TokenType>,
	},
	InvalidSyntax {
		message: String,
		line: usize,
		column: usize,
	},
}

type ParseResult<T> = Result<T, ParseError>;

/// Parser for TSX source code
pub struct Parser<'a> {
	tokens: Peekable<Lexer<'a>>,
}

impl<'a> Parser<'a> {
	/// Creates a new parser for the given input string
	pub fn new(input: &'a str) -> Self {
		let lexer = Lexer::new(input);
		Self { tokens: lexer.peekable() }
	}

	/// Creates a new parser from a pre-existing lexer
	pub fn from_lexer(lexer: Lexer<'a>) -> Self {
		Self { tokens: lexer.peekable() }
	}

	/// Parses the input into an AST
	pub fn parse(&mut self) -> ParseResult<Node> {
		self.parse_program()
	}

	/// Peeks at the next token without consuming it
	fn peek_token(&mut self) -> Option<&Token> {
		self.tokens.peek()
	}

	/// Consumes and returns the next token
	fn next_token(&mut self) -> Option<Token> {
		self.tokens.next()
	}

	/// Checks if the next token matches the expected type
	fn check_token(&mut self, expected: TokenType) -> bool {
		if let Some(token) = self.peek_token() {
			token.token_type == expected
		} else {
			false
		}
	}

	/// Expects a token of a specific type, consuming it if matched
	fn expect_token(&mut self, expected: TokenType) -> ParseResult<Token> {
		if let Some(token) = self.next_token() {
			if token.token_type == expected {
				Ok(token)
			} else {
				Err(ParseError::UnexpectedToken {
					expected: vec![expected],
					found: token.token_type,
					line: token.line,
					column: token.column,
				})
			}
		} else {
			Err(ParseError::UnexpectedEOF { expected: vec![expected] })
		}
	}

	/// Tries to match and consume a token of a specific type
	fn match_token(&mut self, expected: TokenType) -> bool {
		if self.check_token(expected) {
			self.next_token();
			true
		} else {
			false
		}
	}

	/// Parses a complete program
	fn parse_program(&mut self) -> ParseResult<Node> {
		let mut body = Vec::new();

		while self.peek_token().is_some() {
			let statement = self.parse_statement()?;
			body.push(statement);
		}

		Ok(Node::Program { body })
	}

	/// Parses a statement
	fn parse_statement(&mut self) -> ParseResult<Node> {
		match self.peek_token() {
			Some(token) => match token.token_type {
				TokenType::KeywordLet | TokenType::KeywordConst | TokenType::KeywordVar => self.parse_variable_declaration(),
				TokenType::KeywordFunction => self.parse_function_declaration(),
				TokenType::KeywordIf => self.parse_if_statement(),
				TokenType::KeywordReturn => self.parse_return_statement(),
				TokenType::KeywordImport => self.parse_import_declaration(),
				TokenType::KeywordExport => self.parse_export_declaration(),
				TokenType::BraceL => self.parse_block_statement(),
				TokenType::Lt => self.parse_jsx_or_less_than(),
				_ => self.parse_expression_statement(),
			},
			None => Err(ParseError::UnexpectedEOF {
				expected: vec![
					TokenType::KeywordLet,
					TokenType::KeywordConst,
					TokenType::KeywordVar,
					TokenType::KeywordFunction,
					TokenType::KeywordIf,
					TokenType::KeywordReturn,
					TokenType::BraceL,
				],
			}),
		}
	}

	/// Parses a variable declaration (let, const, var)
	fn parse_variable_declaration(&mut self) -> ParseResult<Node> {
		let token = self.next_token().unwrap();
		let kind = match token.token_type {
			TokenType::KeywordLet => VariableKind::Let,
			TokenType::KeywordConst => VariableKind::Const,
			TokenType::KeywordVar => VariableKind::Var,
			_ => unreachable!("Expected variable declaration keyword"),
		};

		let mut declarations = Vec::new();

		// Parse first declarator
		declarations.push(self.parse_variable_declarator()?);

		// Parse additional declarators separated by commas
		while self.match_token(TokenType::Comma) {
			declarations.push(self.parse_variable_declarator()?);
		}

		// Expect semicolon at the end
		self.expect_token(TokenType::Semicolon)?;

		Ok(Node::VariableDeclaration { kind, declarations })
	}

	/// Parses a variable declarator (identifier = initializer)
	fn parse_variable_declarator(&mut self) -> ParseResult<Node> {
		let id = Box::new(self.parse_identifier()?);

		let init = if self.match_token(TokenType::Eq) {
			Some(Box::new(self.parse_expression()?))
		} else {
			None
		};

		Ok(Node::VariableDeclarator { id, init })
	}

	/// Parses an identifier
	fn parse_identifier(&mut self) -> ParseResult<Node> {
		if let Some(token) = self.next_token() {
			match token.token_type {
				TokenType::Identifier => Ok(Node::Identifier { name: token.literal }),
				TokenType::JSXIdentifier => Ok(Node::JSXIdentifier { name: token.literal }),
				_ => Err(ParseError::UnexpectedToken {
					expected: vec![TokenType::Identifier],
					found: token.token_type,
					line: token.line,
					column: token.column,
				}),
			}
		} else {
			Err(ParseError::UnexpectedEOF {
				expected: vec![TokenType::Identifier],
			})
		}
	}

	/// Parses a function declaration
	fn parse_function_declaration(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::KeywordFunction)?;

		// Function name is optional (for anonymous functions)
		let id = if self.check_token(TokenType::Identifier) {
			Some(Box::new(self.parse_identifier()?))
		} else {
			None
		};

		// Parse parameters
		self.expect_token(TokenType::ParenL)?;
		let params = self.parse_function_parameters()?;
		self.expect_token(TokenType::ParenR)?;

		// Parse function body
		let body = Box::new(self.parse_block_statement()?);

		Ok(Node::FunctionDeclaration {
			id,
			params,
			body,
			async_: false,    // We're not handling async functions in this example
			generator: false, // We're not handling generator functions in this example
		})
	}

	/// Parses function parameters
	fn parse_function_parameters(&mut self) -> ParseResult<Vec<Node>> {
		let mut params = Vec::new();

		// Empty parameter list
		if self.check_token(TokenType::ParenR) {
			return Ok(params);
		}

		// First parameter
		params.push(self.parse_identifier()?);

		// Additional parameters separated by commas
		while self.match_token(TokenType::Comma) {
			params.push(self.parse_identifier()?);
		}

		Ok(params)
	}

	/// Parses a block statement
	fn parse_block_statement(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::BraceL)?;

		let mut body = Vec::new();

		while !self.check_token(TokenType::BraceR) && self.peek_token().is_some() {
			body.push(self.parse_statement()?);
		}

		self.expect_token(TokenType::BraceR)?;

		Ok(Node::BlockStatement { body })
	}

	/// Parses an if statement
	fn parse_if_statement(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::KeywordIf)?;

		// Parse condition
		self.expect_token(TokenType::ParenL)?;
		let test = Box::new(self.parse_expression()?);
		self.expect_token(TokenType::ParenR)?;

		// Parse consequent (if body)
		let consequent = Box::new(self.parse_statement()?);

		// Parse alternate (else body) if present
		let alternate = if self.match_token(TokenType::KeywordElse) {
			Some(Box::new(self.parse_statement()?))
		} else {
			None
		};

		Ok(Node::IfStatement { test, consequent, alternate })
	}

	/// Parses a return statement
	fn parse_return_statement(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::KeywordReturn)?;

		let argument = if !self.check_token(TokenType::Semicolon) {
			Some(Box::new(self.parse_expression()?))
		} else {
			None
		};

		self.expect_token(TokenType::Semicolon)?;

		Ok(Node::ReturnStatement { argument })
	}

	/// Parses an expression statement
	fn parse_expression_statement(&mut self) -> ParseResult<Node> {
		let expression = Box::new(self.parse_expression()?);

		self.expect_token(TokenType::Semicolon)?;

		Ok(Node::ExpressionStatement { expression })
	}

	/// Parses an expression
	fn parse_expression(&mut self) -> ParseResult<Node> {
		self.parse_assignment_expression()
	}

	/// Parses an assignment expression
	fn parse_assignment_expression(&mut self) -> ParseResult<Node> {
		let left = self.parse_binary_expression()?;

		match self.peek_token() {
			Some(token) => match token.token_type {
				TokenType::Eq => {
					self.next_token();
					let right = self.parse_assignment_expression()?;
					Ok(Node::AssignmentExpression {
						operator: AssignmentOperator::Equals,
						left: Box::new(left),
						right: Box::new(right),
					})
				}
				TokenType::PlusEq => {
					self.next_token();
					let right = self.parse_assignment_expression()?;
					Ok(Node::AssignmentExpression {
						operator: AssignmentOperator::PlusEquals,
						left: Box::new(left),
						right: Box::new(right),
					})
				}
				TokenType::MinusEq => {
					self.next_token();
					let right = self.parse_assignment_expression()?;
					Ok(Node::AssignmentExpression {
						operator: AssignmentOperator::MinusEquals,
						left: Box::new(left),
						right: Box::new(right),
					})
				}
				TokenType::StarEq => {
					self.next_token();
					let right = self.parse_assignment_expression()?;
					Ok(Node::AssignmentExpression {
						operator: AssignmentOperator::TimesEquals,
						left: Box::new(left),
						right: Box::new(right),
					})
				}
				TokenType::SlashEq => {
					self.next_token();
					let right = self.parse_assignment_expression()?;
					Ok(Node::AssignmentExpression {
						operator: AssignmentOperator::DivideEquals,
						left: Box::new(left),
						right: Box::new(right),
					})
				}
				_ => Ok(left),
			},
			None => Ok(left),
		}
	}

	/// Parses a binary expression
	fn parse_binary_expression(&mut self) -> ParseResult<Node> {
		let mut left = self.parse_primary_expression()?;

		while let Some(token) = self.peek_token() {
			let operator = match token.token_type {
				TokenType::EqEq => BinaryOperator::Equal,
				TokenType::NotEq => BinaryOperator::NotEqual,
				TokenType::EqEqEq => BinaryOperator::StrictEqual,
				TokenType::NotEqEq => BinaryOperator::StrictNotEqual,
				TokenType::Lt => BinaryOperator::LessThan,
				TokenType::LtEq => BinaryOperator::LessThanEqual,
				TokenType::Gt => BinaryOperator::GreaterThan,
				TokenType::GtEq => BinaryOperator::GreaterThanEqual,
				TokenType::Plus => BinaryOperator::Add,
				TokenType::Minus => BinaryOperator::Subtract,
				TokenType::Star => BinaryOperator::Multiply,
				TokenType::Slash => BinaryOperator::Divide,
				TokenType::AmpAmp => BinaryOperator::And,
				TokenType::PipePipe => BinaryOperator::Or,
				_ => break,
			};

			self.next_token();
			let right = self.parse_primary_expression()?;

			left = Node::BinaryExpression {
				operator,
				left: Box::new(left),
				right: Box::new(right),
			};
		}

		Ok(left)
	}

	/// Parses a primary expression (identifiers, literals, etc.)
	fn parse_primary_expression(&mut self) -> ParseResult<Node> {
		match self.peek_token() {
			Some(token) => match token.token_type {
				TokenType::Identifier => self.parse_identifier(),
				TokenType::String => {
					let token = self.next_token().unwrap();
					Ok(Node::Literal {
						value: LiteralValue::String(token.literal),
					})
				}
				TokenType::Number => {
					let token = self.next_token().unwrap();
					let value = token.literal.parse::<f64>().unwrap_or(0.0);
					Ok(Node::Literal {
						value: LiteralValue::Number(value),
					})
				}
				TokenType::KeywordTrue => {
					self.next_token();
					Ok(Node::Literal {
						value: LiteralValue::Boolean(true),
					})
				}
				TokenType::KeywordFalse => {
					self.next_token();
					Ok(Node::Literal {
						value: LiteralValue::Boolean(false),
					})
				}
				TokenType::KeywordNull => {
					self.next_token();
					Ok(Node::Literal { value: LiteralValue::Null })
				}
				TokenType::KeywordUndefined => {
					self.next_token();
					Ok(Node::Literal { value: LiteralValue::Undefined })
				}
				TokenType::ParenL => {
					self.next_token();
					let expr = self.parse_expression()?;
					self.expect_token(TokenType::ParenR)?;
					Ok(expr)
				}
				_ => Err(ParseError::UnexpectedToken {
					expected: vec![
						TokenType::Identifier,
						TokenType::String,
						TokenType::Number,
						TokenType::KeywordTrue,
						TokenType::KeywordFalse,
						TokenType::KeywordNull,
						TokenType::ParenL,
					],
					found: token.token_type,
					line: token.line,
					column: token.column,
				}),
			},
			None => Err(ParseError::UnexpectedEOF {
				expected: vec![
					TokenType::Identifier,
					TokenType::String,
					TokenType::Number,
					TokenType::KeywordTrue,
					TokenType::KeywordFalse,
					TokenType::KeywordNull,
					TokenType::ParenL,
				],
			}),
		}
	}

	/// Determines if we're parsing a JSX element or a less-than operator
	fn parse_jsx_or_less_than(&mut self) -> ParseResult<Node> {
		// We have a '<' token, which could either be a JSX element or a binary expression
		let saved_tokens_state = self.tokens.clone();

		// Consume '<'
		self.next_token();

		// Check if the next token indicates a JSX tag
		match self.peek_token() {
			Some(token) => match token.token_type {
				TokenType::JSXIdentifier => {
					// Looks like JSX, restore the state and parse as JSX
					self.tokens = saved_tokens_state;
					self.parse_jsx_element()
				}
				TokenType::Slash => {
					// JSX closing tag, restore the state and parse as JSX
					self.tokens = saved_tokens_state;
					self.parse_jsx_element()
				}
				_ => {
					// Not JSX, restore the state and parse as binary expression
					self.tokens = saved_tokens_state;
					self.parse_expression_statement()
				}
			},
			None => Err(ParseError::UnexpectedEOF {
				expected: vec![TokenType::JSXIdentifier, TokenType::Slash],
			}),
		}
	}

	/// Parses a JSX element
	fn parse_jsx_element(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::Lt)?;

		// Handle JSX fragment
		if self.check_token(TokenType::Gt) {
			return self.parse_jsx_fragment();
		}

		let opening_element = Box::new(self.parse_jsx_opening_element()?);

		// If self-closing tag, no children or closing tag
		if let Node::JSXOpeningElement { self_closing, .. } = *opening_element {
			if self_closing {
				return Ok(Node::JSXElement {
					opening_element,
					children: Vec::new(),
					closing_element: None,
				});
			}
		}

		// Parse children
		let mut children = Vec::new();

		while !self.check_token(TokenType::JSXClosingElementStart) && self.peek_token().is_some() {
			if self.check_token(TokenType::Lt) {
				// Nested JSX element
				children.push(self.parse_jsx_element()?);
			} else {
				// JSX text content
				let content = self.parse_jsx_text()?;
				if !content.is_empty() {
					children.push(Node::JSXText { value: content });
				}
			}
		}

		// Parse closing tag
		let closing_element = Some(Box::new(self.parse_jsx_closing_element()?));

		Ok(Node::JSXElement {
			opening_element,
			children,
			closing_element,
		})
	}

	/// Parses a JSX opening element
	fn parse_jsx_opening_element(&mut self) -> ParseResult<Node> {
		// The '<' token has already been consumed

		let name = Box::new(self.parse_identifier()?);
		let mut attributes = Vec::new();

		// Parse attributes
		while !self.check_token(TokenType::Gt) && !self.check_token(TokenType::Slash) && self.peek_token().is_some() {
			attributes.push(self.parse_jsx_attribute()?);
		}

		// Check if self-closing
		let self_closing = self.match_token(TokenType::Slash);

		self.expect_token(TokenType::Gt)?;

		Ok(Node::JSXOpeningElement { name, attributes, self_closing })
	}

	/// Parses a JSX closing element
	fn parse_jsx_closing_element(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::JSXClosingElementStart)?;

		let name = Box::new(self.parse_identifier()?);

		self.expect_token(TokenType::Gt)?;

		Ok(Node::JSXClosingElement { name })
	}

	/// Parses a JSX attribute
	fn parse_jsx_attribute(&mut self) -> ParseResult<Node> {
		let name_token = self.expect_token(TokenType::JSXAttributeName)?;
		let name = name_token.literal;

		let value = if self.match_token(TokenType::Eq) {
			if self.check_token(TokenType::JSXAttributeStringValue) {
				let token = self.next_token().unwrap();
				Some(Box::new(Node::Literal {
					value: LiteralValue::String(token.literal),
				}))
			} else if self.match_token(TokenType::BraceL) {
				let expr = self.parse_expression()?;
				self.expect_token(TokenType::BraceR)?;
				Some(Box::new(expr))
			} else {
				return Err(ParseError::UnexpectedToken {
					expected: vec![TokenType::String, TokenType::BraceL],
					found: self.peek_token().unwrap().token_type,
					line: self.peek_token().unwrap().line,
					column: self.peek_token().unwrap().column,
				});
			}
		} else {
			None
		};

		Ok(Node::JSXAttribute { name, value })
	}

	/// Parses JSX text content
	fn parse_jsx_text(&mut self) -> ParseResult<String> {
		let mut text = String::new();

		while let Some(token) = self.peek_token() {
			if token.token_type == TokenType::Lt || token.token_type == TokenType::JSXClosingElementStart {
				break;
			}
			let token = self.next_token().unwrap();
			text.push_str(&token.literal);
		}

		// Trim whitespace for better AST representation
		Ok(text.to_string())
	}

	/// Parses a JSX fragment (<>...</>)
	fn parse_jsx_fragment(&mut self) -> ParseResult<Node> {
		// The '<' token has already been consumed
		self.expect_token(TokenType::Gt)?;

		let mut children = Vec::new();

		while !self.check_token(TokenType::JSXClosingElementStart) && self.peek_token().is_some() {
			if self.check_token(TokenType::Lt) {
				children.push(self.parse_jsx_element()?);
			} else {
				let text = self.parse_jsx_text()?;
				if !text.is_empty() {
					children.push(Node::JSXText { value: text });
				}
			}
		}

		self.expect_token(TokenType::JSXClosingElementStart)?;
		self.expect_token(TokenType::Gt)?;

		Ok(Node::JSXElement {
			opening_element: Box::new(Node::JSXOpeningElement {
				name: Box::new(Node::Identifier { name: "Fragment".to_string() }),
				attributes: Vec::new(),
				self_closing: false,
			}),
			children,
			closing_element: Some(Box::new(Node::JSXClosingElement {
				name: Box::new(Node::Identifier { name: "Fragment".to_string() }),
			})),
		})
	}

	/// Parses an import declaration
	fn parse_import_declaration(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::KeywordImport)?;

		// Parse import specifiers
		let mut specifiers = Vec::new();

		// Default import
		if self.check_token(TokenType::Identifier) {
			let local = Box::new(self.parse_identifier()?);
			let imported = local.clone();
			specifiers.push(Node::ImportSpecifier { imported, local });

			// Named imports
			if self.match_token(TokenType::Comma) {
				self.expect_token(TokenType::BraceL)?;
				self.parse_named_imports(&mut specifiers)?;
				self.expect_token(TokenType::BraceR)?;
			}
		}
		// Named imports only
		else if self.match_token(TokenType::BraceL) {
			self.parse_named_imports(&mut specifiers)?;
			self.expect_token(TokenType::BraceR)?;
		}

		// From clause
		self.expect_token(TokenType::KeywordFrom)?;

		let source_token = self.expect_token(TokenType::String)?;
		let source = Box::new(Node::Literal {
			value: LiteralValue::String(source_token.literal),
		});

		self.expect_token(TokenType::Semicolon)?;

		Ok(Node::ImportDeclaration { specifiers, source })
	}

	/// Helper function to parse named imports
	fn parse_named_imports(&mut self, specifiers: &mut Vec<Node>) -> ParseResult<()> {
		if self.check_token(TokenType::BraceR) {
			return Ok(());
		}

		// First named import
		self.parse_named_import(specifiers)?;

		// Additional named imports
		while self.match_token(TokenType::Comma) && !self.check_token(TokenType::BraceR) {
			self.parse_named_import(specifiers)?;
		}

		Ok(())
	}

	/// Helper function to parse a single named import
	fn parse_named_import(&mut self, specifiers: &mut Vec<Node>) -> ParseResult<()> {
		let imported = Box::new(self.parse_identifier()?);

		let local = if self.match_token(TokenType::KeywordAs) {
			Box::new(self.parse_identifier()?)
		} else {
			imported.clone()
		};

		specifiers.push(Node::ImportSpecifier { imported, local });

		Ok(())
	}

	/// Parses an export declaration
	fn parse_export_declaration(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::KeywordExport)?;

		let declaration = Box::new(self.parse_statement()?);

		Ok(Node::ExportDeclaration { declaration })
	}

	fn parse_type_annotation(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::Colon)?;

		match self.peek_token() {
			Some(token) => match token.token_type {
				TokenType::KeywordString => {
					self.next_token();
					Ok(Node::Identifier { name: "string".to_string() })
				}
				TokenType::KeywordNumber => {
					self.next_token();
					Ok(Node::Identifier { name: "number".to_string() })
				}
				TokenType::KeywordBoolean => {
					self.next_token();
					Ok(Node::Identifier { name: "boolean".to_string() })
				}
				TokenType::KeywordAny => {
					self.next_token();
					Ok(Node::Identifier { name: "any".to_string() })
				}
				TokenType::KeywordVoidType => {
					self.next_token();
					Ok(Node::Identifier { name: "void".to_string() })
				}
				TokenType::Identifier => self.parse_identifier(),
				TokenType::BraceL => self.parse_object_type(),
				TokenType::BracketL => self.parse_array_type(),
				TokenType::ParenL => self.parse_function_type(),
				_ => Err(ParseError::UnexpectedToken {
					expected: vec![
						TokenType::KeywordString,
						TokenType::KeywordNumber,
						TokenType::KeywordBoolean,
						TokenType::KeywordAny,
						TokenType::KeywordVoidType,
						TokenType::Identifier,
						TokenType::BraceL,
						TokenType::BracketL,
						TokenType::ParenL,
					],
					found: token.token_type,
					line: token.line,
					column: token.column,
				}),
			},
			None => Err(ParseError::UnexpectedEOF {
				expected: vec![
					TokenType::KeywordString,
					TokenType::KeywordNumber,
					TokenType::KeywordBoolean,
					TokenType::KeywordAny,
					TokenType::KeywordVoidType,
					TokenType::Identifier,
					TokenType::BraceL,
					TokenType::BracketL,
					TokenType::ParenL,
				],
			}),
		}
	}

	fn parse_object_type(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::BraceL)?;

		// For now, we'll just consume tokens until we find the closing brace
		// A full implementation would parse property types
		let mut depth = 1;

		while depth > 0 && self.peek_token().is_some() {
			let token = self.next_token().unwrap();
			match token.token_type {
				TokenType::BraceL => depth += 1,
				TokenType::BraceR => depth -= 1,
				_ => {}
			}
		}

		// Simple placeholder for object type
		Ok(Node::Identifier { name: "Object".to_string() })
	}

	fn parse_array_type(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::BracketL)?;

		// Parse the element type
		let element_type = self.parse_type_annotation()?;

		self.expect_token(TokenType::BracketR)?;

		// For simplicity, we're just returning a string representation
		let type_name = match element_type {
			Node::Identifier { name } => format!("{}[]", name),
			_ => "Array".to_string(),
		};

		Ok(Node::Identifier { name: type_name })
	}

	/// Parses a function type
	fn parse_function_type(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::ParenL)?;

		// For now, we'll just consume tokens until we find the closing parenthesis
		// A full implementation would parse parameter types
		let mut depth = 1;

		while depth > 0 && self.peek_token().is_some() {
			let token = self.next_token().unwrap();
			match token.token_type {
				TokenType::ParenL => depth += 1,
				TokenType::ParenR => depth -= 1,
				_ => {}
			}
		}

		// Expect the return type arrow
		self.expect_token(TokenType::EqEq)?;
		self.expect_token(TokenType::Gt)?;

		// Parse the return type
		let return_type = self.parse_type_annotation()?;

		// For simplicity, we're just returning a string representation
		let type_name = match return_type {
			Node::Identifier { name } => format!("Function => {}", name),
			_ => "Function".to_string(),
		};

		Ok(Node::Identifier { name: type_name })
	}

	/// Parses a type alias declaration
	// TODO: remove when used
	#[allow(dead_code)]
	fn parse_type_alias(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::KeywordType)?;

		let name = self.parse_identifier()?;

		self.expect_token(TokenType::Eq)?;

		let type_annotation = self.parse_type_annotation()?;

		self.expect_token(TokenType::Semicolon)?;

		// For now, we'll just return a simple representation
		// A full implementation would create a proper AST node
		Ok(Node::ExpressionStatement {
			expression: Box::new(Node::AssignmentExpression {
				operator: AssignmentOperator::Equals,
				left: Box::new(name),
				right: Box::new(type_annotation),
			}),
		})
	}

	// TODO: remove when used
	#[allow(dead_code)]
	fn parse_interface_declaration(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::KeywordInterface)?;

		let name = self.parse_identifier()?;

		// Handle potential extends clause
		if self.match_token(TokenType::KeywordExtends) {
			// In a full implementation, we would parse the extended interfaces
			self.parse_identifier()?;
		}

		// Parse interface body
		self.expect_token(TokenType::BraceL)?;

		// For now, we'll just consume tokens until we find the closing brace
		// A full implementation would parse method and property declarations
		let mut depth = 1;

		while depth > 0 && self.peek_token().is_some() {
			let token = self.next_token().unwrap();
			match token.token_type {
				TokenType::BraceL => depth += 1,
				TokenType::BraceR => depth -= 1,
				_ => {}
			}
		}

		// For now, we'll just return a simple representation
		// A full implementation would create a proper AST node
		Ok(Node::ExpressionStatement { expression: Box::new(name) })
	}

	// TODO: remove when used
	#[allow(dead_code)]
	fn parse_async_function_declaration(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::KeywordAsync)?;
		self.expect_token(TokenType::KeywordFunction)?;

		// Function name is optional (for anonymous functions)
		let id = if self.check_token(TokenType::Identifier) {
			Some(Box::new(self.parse_identifier()?))
		} else {
			None
		};

		// Parse parameters
		self.expect_token(TokenType::ParenL)?;
		let params = self.parse_function_parameters()?;
		self.expect_token(TokenType::ParenR)?;

		// Parse function body
		let body = Box::new(self.parse_block_statement()?);

		Ok(Node::FunctionDeclaration {
			id,
			params,
			body,
			async_: true, // This is an async function
			generator: false,
		})
	}

	/// Parse await expression
	// TODO: remove when used
	#[allow(dead_code)]
	fn parse_await_expression(&mut self) -> ParseResult<Node> {
		self.expect_token(TokenType::KeywordAwait)?;

		let expression = self.parse_expression()?;

		// In a full implementation, this would have its own node type
		// For now, we'll represent it as a regular expression
		Ok(expression)
	}
}

impl std::fmt::Display for ParseError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::UnexpectedToken { expected, found, line, column } => {
				write!(f, "Unexpected token: found {:?} at line {}, column {}, expected one of {:?}", found, line, column, expected)
			}
			Self::UnexpectedEOF { expected } => {
				write!(f, "Unexpected end of file, expected one of {:?}", expected)
			}
			Self::InvalidSyntax { message, line, column } => {
				write!(f, "Syntax error at line {}, column {}: {}", line, column, message)
			}
		}
	}
}

// /// Extension trait to create a parser from a vector of tokens
// pub trait IntoParser {
// 	fn into_parser(self) -> Parser<'static>;
// }
//
// impl IntoParser for Vec<Token> {
// 	fn into_parser(self) -> Parser<'static> {
// 		Parser {
// 			tokens: self.into_iter().peekable(),
// 		}
// 	}
// }

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_variable_declaration() {
		let input = "let x = 42;";
		let mut parser = Parser::new(input);
		let program = parser.parse().unwrap();

		match program {
			Node::Program { body } => {
				assert_eq!(body.len(), 1);
				match &body[0] {
					Node::VariableDeclaration { kind, declarations } => {
						assert_eq!(*kind, VariableKind::Let);
						assert_eq!(declarations.len(), 1);
						match &declarations[0] {
							Node::VariableDeclarator { id, init } => {
								match &**id {
									Node::Identifier { name } => {
										assert_eq!(name, "x");
									}
									_ => panic!("Expected identifier"),
								}
								match &init {
									Some(expr) => match &**expr {
										Node::Literal { value } => match value {
											LiteralValue::Number(n) => assert_eq!(*n, 42.0),
											_ => panic!("Expected number literal"),
										},
										_ => panic!("Expected literal"),
									},
									None => panic!("Expected initializer"),
								}
							}
							_ => panic!("Expected variable declarator"),
						}
					}
					_ => panic!("Expected variable declaration"),
				}
			}
			_ => panic!("Expected program"),
		}
	}

	#[test]
	fn test_parse_function_declaration() {
		let input = "function add(a, b) { return a + b; }";
		let mut parser = Parser::new(input);
		let program = parser.parse().unwrap();

		match program {
			Node::Program { body } => {
				assert_eq!(body.len(), 1);
				match &body[0] {
					Node::FunctionDeclaration {
						id,
						params,
						body,
						async_,
						generator,
					} => {
						assert!(!*async_);
						assert!(!*generator);

						// Check function name
						match &**id.as_ref().unwrap() {
							Node::Identifier { name } => assert_eq!(name, "add"),
							_ => panic!("Expected identifier"),
						}

						// Check parameters
						assert_eq!(params.len(), 2);
						match &params[0] {
							Node::Identifier { name } => assert_eq!(name, "a"),
							_ => panic!("Expected identifier"),
						}
						match &params[1] {
							Node::Identifier { name } => assert_eq!(name, "b"),
							_ => panic!("Expected identifier"),
						}

						// Check function body
						match &**body {
							Node::BlockStatement { body } => {
								assert_eq!(body.len(), 1);
								match &body[0] {
									Node::ReturnStatement { argument } => match &**argument.as_ref().unwrap() {
										Node::BinaryExpression { operator, left, right } => {
											assert_eq!(*operator, BinaryOperator::Add);

											match &**left {
												Node::Identifier { name } => assert_eq!(name, "a"),
												_ => panic!("Expected identifier"),
											}

											match &**right {
												Node::Identifier { name } => assert_eq!(name, "b"),
												_ => panic!("Expected identifier"),
											}
										}
										_ => panic!("Expected binary expression"),
									},
									_ => panic!("Expected return statement"),
								}
							}
							_ => panic!("Expected block statement"),
						}
					}
					_ => panic!("Expected function declaration"),
				}
			}
			_ => panic!("Expected program"),
		}
	}

	#[test]
	fn test_parse_if_statement() {
		let input = "if (x > 0) { return true; } else { return false; }";
		let mut parser = Parser::new(input);
		let program = parser.parse().unwrap();

		match program {
			Node::Program { body } => {
				assert_eq!(body.len(), 1);
				match &body[0] {
					Node::IfStatement { test, consequent, alternate } => {
						// Check condition
						match &**test {
							Node::BinaryExpression { operator, left, right } => {
								assert_eq!(*operator, BinaryOperator::GreaterThan);

								match &**left {
									Node::Identifier { name } => assert_eq!(name, "x"),
									_ => panic!("Expected identifier"),
								}

								match &**right {
									Node::Literal { value } => match value {
										LiteralValue::Number(n) => assert_eq!(*n, 0.0),
										_ => panic!("Expected number literal"),
									},
									_ => panic!("Expected literal"),
								}
							}
							_ => panic!("Expected binary expression"),
						}

						// Check if-branch
						match &**consequent {
							Node::BlockStatement { body } => {
								assert_eq!(body.len(), 1);
								match &body[0] {
									Node::ReturnStatement { argument } => match &**argument.as_ref().unwrap() {
										Node::Literal { value } => match value {
											LiteralValue::Boolean(b) => assert!(*b),
											_ => panic!("Expected boolean literal"),
										},
										_ => panic!("Expected literal"),
									},
									_ => panic!("Expected return statement"),
								}
							}
							_ => panic!("Expected block statement"),
						}

						// Check else-branch
						match &**alternate.as_ref().unwrap() {
							Node::BlockStatement { body } => {
								assert_eq!(body.len(), 1);
								match &body[0] {
									Node::ReturnStatement { argument } => match &**argument.as_ref().unwrap() {
										Node::Literal { value } => match value {
											LiteralValue::Boolean(b) => assert!(!*b),
											_ => panic!("Expected boolean literal"),
										},
										_ => panic!("Expected literal"),
									},
									_ => panic!("Expected return statement"),
								}
							}
							_ => panic!("Expected block statement"),
						}
					}
					_ => panic!("Expected if statement"),
				}
			}
			_ => panic!("Expected program"),
		}
	}

	#[test]
	fn test_parse_jsx_element() {
		let input = "<div className=\"container\">Hello, world!</div>";
		let mut parser = Parser::new(input);
		let program = parser.parse().unwrap();

		match program {
			Node::Program { body } => {
				assert_eq!(body.len(), 1);
				match &body[0] {
					Node::JSXElement {
						opening_element,
						children,
						closing_element,
					} => {
						// Check opening tag
						match &**opening_element {
							Node::JSXOpeningElement { name, attributes, self_closing } => {
								assert!(!*self_closing);

								match &**name {
									Node::JSXIdentifier { name } => assert_eq!(name, "div"),
									_ => panic!("Expected identifier"),
								}

								assert_eq!(attributes.len(), 1);
								match &attributes[0] {
									Node::JSXAttribute { name, value } => {
										assert_eq!(name, "className");
										match &**value.as_ref().unwrap() {
											Node::Literal { value } => match value {
												LiteralValue::String(s) => assert_eq!(s, "container"),
												_ => panic!("Expected string literal"),
											},
											_ => panic!("Expected literal"),
										}
									}
									_ => panic!("Expected JSX attribute"),
								}
							}
							_ => panic!("Expected JSX opening element"),
						}

						// Check children
						assert_eq!(children.len(), 1);
						match &children[0] {
							Node::JSXText { value } => assert_eq!(value, "Hello, world!"),
							_ => panic!("Expected JSX text"),
						}

						// Check closing tag
						match &**closing_element.as_ref().unwrap() {
							Node::JSXClosingElement { name } => match &**name {
								Node::JSXIdentifier { name } => assert_eq!(name, "div"),
								_ => panic!("Expected identifier"),
							},
							_ => panic!("Expected JSX closing element"),
						}
					}
					_ => panic!("Expected JSX element"),
				}
			}
			_ => panic!("Expected program"),
		}
	}

	#[test]
	fn test_parse_import_declaration() {
		let input = "import React, { useState } from 'react';";
		let mut parser = Parser::new(input);
		let program = parser.parse().unwrap();

		match program {
			Node::Program { body } => {
				assert_eq!(body.len(), 1);
				match &body[0] {
					Node::ImportDeclaration { specifiers, source } => {
						assert_eq!(specifiers.len(), 2);

						// Check default import
						match &specifiers[0] {
							Node::ImportSpecifier { imported, local } => {
								match &**imported {
									Node::Identifier { name } => assert_eq!(name, "React"),
									_ => panic!("Expected identifier"),
								}
								match &**local {
									Node::Identifier { name } => assert_eq!(name, "React"),
									_ => panic!("Expected identifier"),
								}
							}
							_ => panic!("Expected import specifier"),
						}

						// Check named import
						match &specifiers[1] {
							Node::ImportSpecifier { imported, local } => {
								match &**imported {
									Node::Identifier { name } => assert_eq!(name, "useState"),
									_ => panic!("Expected identifier"),
								}
								match &**local {
									Node::Identifier { name } => assert_eq!(name, "useState"),
									_ => panic!("Expected identifier"),
								}
							}
							_ => panic!("Expected import specifier"),
						}

						// Check source
						match &**source {
							Node::Literal { value } => match value {
								LiteralValue::String(s) => assert_eq!(s, "react"),
								_ => panic!("Expected string literal"),
							},
							_ => panic!("Expected literal"),
						}
					}
					_ => panic!("Expected import declaration"),
				}
			}
			_ => panic!("Expected program"),
		}
	}

	#[test]
	fn test_parse_complex_expression() {
		let input = "let result = (a + b) * (c - d / 2);";
		let mut parser = Parser::new(input);
		let program = parser.parse().unwrap();

		// This is just a basic check for successful parsing
		match program {
			Node::Program { body } => {
				assert_eq!(body.len(), 1);
				match &body[0] {
					Node::VariableDeclaration { kind, declarations } => {
						assert_eq!(*kind, VariableKind::Let);
						assert_eq!(declarations.len(), 1);
					}
					_ => panic!("Expected variable declaration"),
				}
			}
			_ => panic!("Expected program"),
		}
	}

	#[test]
	fn test_parse_self_closing_jsx() {
		let input = "<input type=\"text\" disabled />";
		let mut parser = Parser::new(input);
		let program = parser.parse().unwrap();

		match program {
			Node::Program { body } => {
				assert_eq!(body.len(), 1);
				match &body[0] {
					Node::JSXElement {
						opening_element,
						children,
						closing_element,
					} => {
						// Check opening tag
						match &**opening_element {
							Node::JSXOpeningElement { name, attributes, self_closing } => {
								assert!(*self_closing);

								match &**name {
									Node::Identifier { name } => assert_eq!(name, "input"),
									_ => panic!("Expected identifier"),
								}

								assert_eq!(attributes.len(), 2);
								assert!(closing_element.is_none());
								assert_eq!(children.len(), 0);
							}
							_ => panic!("Expected JSX opening element"),
						}
					}
					_ => panic!("Expected JSX element"),
				}
			}
			_ => panic!("Expected program"),
		}
	}
}
