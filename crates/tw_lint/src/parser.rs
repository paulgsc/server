use crate::ast::{AttributeValue, Element, Fragment, JSXAttribute, JSXExpression, Node};
use crate::lexer::{Lexer, Token, TokenType};
use std::iter::Peekable;
use std::vec::IntoIter;

pub struct Parser {
	tokens: Peekable<IntoIter<Token>>,
	current_token: Token,
	peek_token: Token,
}

#[derive(Debug)]
pub enum ParserError {
	UnexpectedToken(Token, &'static str),
	ExpectedToken(TokenType, Token),
	UnexpectedEOF,
	InvalidAttribute,
	UnclosedElement(String),
	UnclosedExpression,
	MismatchedTag(String, String),
	Generic(String),
}

impl Parser {
	#[must_use]
	pub fn new(lexer: Lexer) -> Self {
		let tokens: Vec<Token> = lexer.collect();
		let mut token_iter = tokens.into_iter().peekable();

		let current_token = token_iter.next().unwrap_or(Token::new(TokenType::EOF, String::new(), 0, 0));

		let peek_token = token_iter.peek().cloned().unwrap_or(Token::new(TokenType::EOF, String::new(), 0, 0));

		Self {
			tokens: token_iter,
			current_token,
			peek_token,
		}
	}

	fn next_token(&mut self) {
		self.current_token = self.peek_token.clone();
		self.peek_token = self.tokens.next().unwrap_or(Token::new(TokenType::EOF, String::new(), 0, 0));
	}

	fn expect_token(&mut self, token_type: TokenType) -> Result<Token, ParserError> {
		let current = self.current_token.clone();
		if current.token_type == token_type {
			let token = current;
			self.next_token();
			Ok(token)
		} else {
			Err(ParserError::ExpectedToken(token_type, current))
		}
	}

	fn peek_token_is(&self, token_type: TokenType) -> bool {
		self.peek_token.token_type == token_type
	}

	fn current_token_is(&self, token_type: TokenType) -> bool {
		self.current_token.token_type == token_type
	}

	pub fn parse(&mut self) -> Result<Vec<Node>, ParserError> {
		let mut nodes = Vec::new();

		self.next_token(); // Initialize the first token

		while self.current_token.token_type != TokenType::EOF {
			let node = self.parse_node()?;
			nodes.push(node);
		}

		Ok(nodes)
	}

	fn parse_node(&mut self) -> Result<Node, ParserError> {
		match self.current_token.token_type {
			TokenType::Lt => self.parse_jsx_element_or_fragment(),
			TokenType::BraceL => self.parse_jsx_expression_node(),
			// Add other token types as needed
			_ => {
				// Assume text node for anything else
				let text = self.current_token.literal.clone();
				self.next_token();
				Ok(Node::Text(text))
			}
		}
	}

	fn parse_jsx_element_or_fragment(&mut self) -> Result<Node, ParserError> {
		// Check for fragment: <>...</>
		if self.peek_token_is(TokenType::Gt) {
			return self.parse_jsx_fragment();
		}

		self.next_token(); // Consume the '<'

		if self.current_token_is(TokenType::JSXClosingElementStart) {
			return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Unexpected closing tag"));
		}

		// Parse the tag name
		let tag_name = if self.current_token_is(TokenType::Identifier) || self.current_token_is(TokenType::JSXIdentifier) {
			self.current_token.literal.clone()
		} else {
			return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Expected tag name"));
		};

		self.next_token(); // Consume the tag name

		// Check for TypeScript generic type arguments
		let type_arguments = if self.current_token_is(TokenType::Lt) {
			Some(self.parse_type_arguments()?)
		} else {
			None
		};

		// Parse attributes
		let mut element = Element::new(&tag_name);
		if type_arguments.is_some() {
			element.set_type_arguments(&type_arguments.unwrap());
		}

		while !self.current_token_is(TokenType::JSXOpeningElementEnd) && !self.current_token_is(TokenType::Gt) && !self.current_token_is(TokenType::EOF) {
			if self.current_token_is(TokenType::JSXSpread) {
				let spread_attr = self.parse_jsx_spread_attribute()?;
				element.attributes.push(spread_attr);
			} else if self.current_token_is(TokenType::Identifier) || self.current_token_is(TokenType::JSXAttributeName) {
				let attr = self.parse_jsx_attribute()?;
				element.attributes.push(attr);
			} else {
				return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Expected attribute or closing of tag"));
			}
		}

		// Check if it's a self-closing tag
		let is_self_closing = self.current_token_is(TokenType::JSXOpeningElementEnd);
		if is_self_closing {
			element.set_self_closing(true);
			self.next_token(); // Consume the '/>'
			return Ok(Node::Element(element));
		}

		// Expect closing '>'
		self.expect_token(TokenType::Gt)?;

		// Parse children
		while !self.current_token_is(TokenType::JSXClosingElementStart) && !self.current_token_is(TokenType::EOF) {
			let child = self.parse_node()?;
			element.add_child(child);
		}

		// Parse closing tag
		self.next_token(); // Consume the '</'
		let closing_tag_name = if self.current_token_is(TokenType::Identifier) || self.current_token_is(TokenType::JSXIdentifier) {
			self.current_token.literal.clone()
		} else {
			return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Expected closing tag name"));
		};

		if tag_name != closing_tag_name {
			return Err(ParserError::MismatchedTag(tag_name, closing_tag_name));
		}

		self.next_token(); // Consume the closing tag name
		self.expect_token(TokenType::Gt)?; // Expect '>'

		Ok(Node::Element(element))
	}

	fn parse_jsx_fragment(&mut self) -> Result<Node, ParserError> {
		self.next_token(); // Consume the '<'
		self.next_token(); // Consume the '>'

		let mut fragment = Fragment { children: Vec::new() };

		// Parse children until we find the closing fragment tag
		while !self.current_token_is(TokenType::JSXClosingElementStart) && !self.current_token_is(TokenType::EOF) {
			let child = self.parse_node()?;
			fragment.children.push(child);
		}

		// Expect '</>'
		self.next_token(); // Consume the '</'
		self.expect_token(TokenType::Gt)?; // Expect '>'

		Ok(Node::Fragment(fragment))
	}

	fn parse_jsx_attribute(&mut self) -> Result<JSXAttribute, ParserError> {
		let name = self.current_token.literal.clone();
		self.next_token(); // Consume the attribute name

		// If there's no '=', it's a boolean attribute
		if !self.current_token_is(TokenType::Eq) {
			return Ok(JSXAttribute {
				name,
				value: Some(AttributeValue::Expression(JSXExpression::BooleanLiteral(true))),
			});
		}

		self.next_token(); // Consume the '='

		let value = match self.current_token.token_type {
			TokenType::String | TokenType::JSXAttributeStringValue => {
				let string_literal = self.current_token.literal.clone();
				self.next_token(); // Consume the string
				Some(AttributeValue::String(string_literal))
			}
			TokenType::BraceL => {
				self.next_token(); // Consume the '{'
				let expr = self.parse_jsx_expression()?;
				self.expect_token(TokenType::BraceR)?; // Expect closing '}'
				Some(AttributeValue::Expression(expr))
			}
			_ => {
				return Err(ParserError::UnexpectedToken(
					self.current_token.clone(),
					"Expected string or expression for attribute value",
				));
			}
		};

		Ok(JSXAttribute { name, value })
	}

	fn parse_jsx_spread_attribute(&mut self) -> Result<JSXAttribute, ParserError> {
		self.next_token(); // Consume the '{'
		self.next_token(); // Consume the '...'

		let expr = self.parse_jsx_expression()?;
		self.expect_token(TokenType::BraceR)?; // Expect closing '}'

		Ok(JSXAttribute {
			name: "...".to_string(),
			value: Some(AttributeValue::Expression(expr)),
		})
	}

	fn parse_jsx_expression_node(&mut self) -> Result<Node, ParserError> {
		self.next_token(); // Consume the '{'
		let expr = self.parse_jsx_expression()?;
		self.expect_token(TokenType::BraceR)?; // Expect closing '}'
		Ok(Node::JSXExpression(expr))
	}

	fn parse_jsx_expression(&mut self) -> Result<JSXExpression, ParserError> {
		match self.current_token.token_type {
			TokenType::Identifier => self.parse_identifier_expression(),
			TokenType::String => {
				let value = self.current_token.literal.clone();
				self.next_token();
				Ok(JSXExpression::StringLiteral(value))
			}
			TokenType::Number => {
				let value = self.current_token.literal.clone();
				self.next_token();
				Ok(JSXExpression::NumberLiteral(value))
			}
			TokenType::KeywordTrue => {
				self.next_token();
				Ok(JSXExpression::BooleanLiteral(true))
			}
			TokenType::KeywordFalse => {
				self.next_token();
				Ok(JSXExpression::BooleanLiteral(false))
			}
			TokenType::KeywordNull => {
				self.next_token();
				Ok(JSXExpression::NullLiteral)
			}
			TokenType::KeywordUndefined => {
				self.next_token();
				Ok(JSXExpression::UndefinedLiteral)
			}
			TokenType::BraceL => self.parse_object_expression(),
			TokenType::BracketL => self.parse_array_expression(),
			TokenType::ParenL => self.parse_grouped_expression(),
			TokenType::Bang | TokenType::Minus | TokenType::Plus => self.parse_unary_expression(),
			TokenType::Lt => {
				// This might be a JSX element inside an expression
				let element = self.parse_jsx_element_or_fragment()?;
				match element {
					Node::Element(el) => Ok(JSXExpression::JSXElement(el)),
					_ => Err(ParserError::Generic("Expected JSX element".to_string())),
				}
			}
			_ => Err(ParserError::UnexpectedToken(self.current_token.clone(), "Unexpected token in expression")),
		}
	}

	fn parse_identifier_expression(&mut self) -> Result<JSXExpression, ParserError> {
		let identifier = self.current_token.literal.clone();
		self.next_token();

		// Check if it's a function call
		if self.current_token_is(TokenType::ParenL) {
			return self.parse_call_expression(identifier);
		}

		// Check if it's a member expression (e.g., obj.prop)
		if self.current_token_is(TokenType::Dot) {
			return self.parse_member_expression(JSXExpression::Identifier(identifier));
		}

		// Check if it's part of a binary expression
		if self.is_binary_operator(self.current_token.token_type) {
			return self.parse_binary_expression(JSXExpression::Identifier(identifier));
		}

		// Otherwise, it's just an identifier
		Ok(JSXExpression::Identifier(identifier))
	}

	fn parse_object_expression(&mut self) -> Result<JSXExpression, ParserError> {
		self.next_token(); // Consume the '{'
		let mut properties = Vec::new();

		// Parse key-value pairs until we find a closing brace
		while !self.current_token_is(TokenType::BraceR) && !self.current_token_is(TokenType::EOF) {
			// Parse key
			let key = if self.current_token_is(TokenType::Identifier) || self.current_token_is(TokenType::String) {
				self.current_token.literal.clone()
			} else {
				return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Expected property name"));
			};

			self.next_token();
			self.expect_token(TokenType::Colon)?;

			// Parse value
			let value = self.parse_jsx_expression()?;
			properties.push((key, Box::new(value)));

			// Check for comma or closing brace
			if self.current_token_is(TokenType::Comma) {
				self.next_token();
			} else if !self.current_token_is(TokenType::BraceR) {
				return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Expected ',' or '}'"));
			}
		}

		self.expect_token(TokenType::BraceR)?;
		Ok(JSXExpression::ObjectExpression(properties))
	}

	fn parse_array_expression(&mut self) -> Result<JSXExpression, ParserError> {
		self.next_token(); // Consume the '['
		let mut elements = Vec::new();

		// Parse elements until we find a closing bracket
		while !self.current_token_is(TokenType::BracketR) && !self.current_token_is(TokenType::EOF) {
			let element = self.parse_jsx_expression()?;
			elements.push(Box::new(element));

			// Check for comma or closing bracket
			if self.current_token_is(TokenType::Comma) {
				self.next_token();
			} else if !self.current_token_is(TokenType::BracketR) {
				return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Expected ',' or ']'"));
			}
		}

		self.expect_token(TokenType::BracketR)?;
		Ok(JSXExpression::ArrayExpression(elements))
	}

	fn parse_call_expression(&mut self, fn_name: String) -> Result<JSXExpression, ParserError> {
		self.next_token(); // Consume the '('
		let mut arguments = Vec::new();

		// Parse arguments until we find a closing parenthesis
		while !self.current_token_is(TokenType::ParenR) && !self.current_token_is(TokenType::EOF) {
			let arg = self.parse_jsx_expression()?;
			arguments.push(arg);

			// Check for comma or closing parenthesis
			if self.current_token_is(TokenType::Comma) {
				self.next_token();
			} else if !self.current_token_is(TokenType::ParenR) {
				return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Expected ',' or ')'"));
			}
		}

		self.expect_token(TokenType::ParenR)?;

		// Check if it's part of a member expression or binary expression
		if self.current_token_is(TokenType::Dot) {
			return self.parse_member_expression(JSXExpression::CallExpression { name: fn_name, arguments });
		}

		if self.is_binary_operator(self.current_token.token_type) {
			return self.parse_binary_expression(JSXExpression::CallExpression { name: fn_name, arguments });
		}

		Ok(JSXExpression::CallExpression { name: fn_name, arguments })
	}

	fn parse_member_expression(&mut self, object: JSXExpression) -> Result<JSXExpression, ParserError> {
		self.next_token(); // Consume the '.'

		// Expect property name
		let property = if self.current_token_is(TokenType::Identifier) {
			self.current_token.literal.clone()
		} else {
			return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Expected property name"));
		};

		self.next_token();

		let expr = JSXExpression::MemberExpression {
			object: Box::new(object),
			property,
		};

		// Check for chained member access (e.g., obj.prop1.prop2)
		if self.current_token_is(TokenType::Dot) {
			return self.parse_member_expression(expr);
		}

		// Check if it's part of a binary expression
		if self.is_binary_operator(self.current_token.token_type) {
			return self.parse_binary_expression(expr);
		}

		Ok(expr)
	}

	fn parse_binary_expression(&mut self, left: JSXExpression) -> Result<JSXExpression, ParserError> {
		let operator = self.current_token.literal.clone();
		let precedence = self.get_precedence(self.current_token.token_type);

		self.next_token();
		let right = self.parse_jsx_expression_with_precedence(precedence)?;

		Ok(JSXExpression::BinaryExpression {
			operator,
			left: Box::new(left),
			right: Box::new(right),
		})
	}

	fn parse_unary_expression(&mut self) -> Result<JSXExpression, ParserError> {
		let operator = self.current_token.literal.clone();
		self.next_token();

		let argument = self.parse_jsx_expression()?;

		Ok(JSXExpression::UnaryExpression {
			operator,
			argument: Box::new(argument),
		})
	}

	fn parse_grouped_expression(&mut self) -> Result<JSXExpression, ParserError> {
		self.next_token(); // Consume the '('

		// Check if it's an arrow function
		if self.current_token_is(TokenType::ParenR) || self.current_token_is(TokenType::Identifier) {
			return self.parse_arrow_function();
		}

		let expr = self.parse_jsx_expression()?;
		self.expect_token(TokenType::ParenR)?;

		// Check if it's a conditional expression
		if self.current_token_is(TokenType::Question) {
			return self.parse_conditional_expression(expr);
		}

		// Check if it's part of a member expression
		if self.current_token_is(TokenType::Dot) {
			return self.parse_member_expression(expr);
		}

		// Check if it's part of a binary expression
		if self.is_binary_operator(self.current_token.token_type) {
			return self.parse_binary_expression(expr);
		}

		Ok(expr)
	}

	fn parse_arrow_function(&mut self) -> Result<JSXExpression, ParserError> {
		let mut parameters = Vec::new();

		// Parse parameters
		if !self.current_token_is(TokenType::ParenR) {
			while !self.current_token_is(TokenType::ParenR) && !self.current_token_is(TokenType::EOF) {
				if self.current_token_is(TokenType::Identifier) {
					parameters.push(self.current_token.literal.clone());
					self.next_token();

					if self.current_token_is(TokenType::Comma) {
						self.next_token();
					} else if !self.current_token_is(TokenType::ParenR) {
						return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Expected ',' or ')'"));
					}
				} else {
					return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Expected parameter name"));
				}
			}
		}

		self.expect_token(TokenType::ParenR)?;

		// Expect '=>'
		if !(self.current_token_is(TokenType::Eq) && self.peek_token_is(TokenType::Gt)) {
			return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Expected '=>'"));
		}

		self.next_token(); // Consume '='
		self.next_token(); // Consume '>'

		// Parse body
		let body = if self.current_token_is(TokenType::BraceL) {
			// Block body
			self.next_token(); // Consume '{'
			let mut statements = Vec::new();

			while !self.current_token_is(TokenType::BraceR) && !self.current_token_is(TokenType::EOF) {
				let stmt = self.parse_node()?;
				statements.push(stmt);
			}

			self.expect_token(TokenType::BraceR)?;
			Box::new(JSXExpression::BlockStatement(statements))
		} else {
			// Expression body
			Box::new(self.parse_jsx_expression()?)
		};

		Ok(JSXExpression::ArrowFunctionExpression { parameters, body })
	}

	fn parse_conditional_expression(&mut self, condition: JSXExpression) -> Result<JSXExpression, ParserError> {
		self.next_token(); // Consume the '?'

		let consequent = self.parse_jsx_expression()?;

		self.expect_token(TokenType::Colon)?;

		let alternate = self.parse_jsx_expression()?;

		Ok(JSXExpression::ConditionalExpression {
			condition: Box::new(condition),
			consequent: Box::new(consequent),
			alternate: Box::new(alternate),
		})
	}

	fn parse_jsx_expression_with_precedence(&mut self, precedence: u8) -> Result<JSXExpression, ParserError> {
		let mut left = match self.current_token.token_type {
			TokenType::Identifier => self.parse_identifier_expression()?,
			TokenType::String => {
				let value = self.current_token.literal.clone();
				self.next_token();
				JSXExpression::StringLiteral(value)
			}
			TokenType::Number => {
				let value = self.current_token.literal.clone();
				self.next_token();
				JSXExpression::NumberLiteral(value)
			}
			TokenType::KeywordTrue => {
				self.next_token();
				JSXExpression::BooleanLiteral(true)
			}
			TokenType::KeywordFalse => {
				self.next_token();
				JSXExpression::BooleanLiteral(false)
			}
			TokenType::KeywordNull => {
				self.next_token();
				JSXExpression::NullLiteral
			}
			TokenType::KeywordUndefined => {
				self.next_token();
				JSXExpression::UndefinedLiteral
			}
			TokenType::BraceL => self.parse_object_expression()?,
			TokenType::BracketL => self.parse_array_expression()?,
			TokenType::ParenL => self.parse_grouped_expression()?,
			TokenType::Bang | TokenType::Minus | TokenType::Plus => self.parse_unary_expression()?,
			_ => return Err(ParserError::UnexpectedToken(self.current_token.clone(), "Unexpected token in expression")),
		};

		while !self.current_token_is(TokenType::EOF) && precedence < self.get_precedence(self.current_token.token_type) {
			if self.current_token_is(TokenType::Dot) {
				left = self.parse_member_expression(left)?;
			} else if self.is_binary_operator(self.current_token.token_type) {
				left = self.parse_binary_expression(left)?;
			} else if self.current_token_is(TokenType::Question) {
				left = self.parse_conditional_expression(left)?;
			} else {
				break;
			}
		}

		Ok(left)
	}

	const fn is_binary_operator(&self, token_type: TokenType) -> bool {
		matches!(
			token_type,
			TokenType::Plus
				| TokenType::Minus
				| TokenType::Star
				| TokenType::Slash
				| TokenType::EqEq
				| TokenType::NotEq
				| TokenType::Lt
				| TokenType::Gt
				| TokenType::LtEq
				| TokenType::GtEq
				| TokenType::AmpAmp
				| TokenType::PipePipe
				| TokenType::EqEqEq
				| TokenType::NotEqEq
		)
	}

	const fn get_precedence(&self, token_type: TokenType) -> u8 {
		match token_type {
			TokenType::PipePipe => 1,
			TokenType::AmpAmp => 2,
			TokenType::EqEq | TokenType::NotEq | TokenType::EqEqEq | TokenType::NotEqEq => 3,
			TokenType::Lt | TokenType::Gt | TokenType::LtEq | TokenType::GtEq => 4,
			TokenType::Plus | TokenType::Minus => 5,
			TokenType::Star | TokenType::Slash => 6,
			_ => 0,
		}
	}

	fn parse_type_arguments(&mut self) -> Result<String, ParserError> {
		let mut type_args = String::new();
		let mut depth = 1;

		type_args.push('<');
		self.next_token(); // Consume the '<'

		while depth > 0 && !self.current_token_is(TokenType::EOF) {
			if self.current_token_is(TokenType::Lt) {
				depth += 1;
			} else if self.current_token_is(TokenType::Gt) {
				depth -= 1;
				if depth == 0 {
					type_args.push('>');
					self.next_token(); // Consume the closing '>'
					break;
				}
			}

			type_args.push_str(&self.current_token.literal);
			self.next_token();
		}

		if depth > 0 {
			return Err(ParserError::Generic("Unclosed type arguments".to_string()));
		}

		Ok(type_args)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::lexer::Lexer;

	#[test]
	fn test_parse_no_err() {
		let input = "<div className=\"container\">Hello, world!</div>";
		let lexer = Lexer::new(input);
		let mut parser = Parser::new(lexer);

		let result = parser.parse();
		assert!(result.is_ok(), "Expected Ok(_), got {:?}", result);
	}

	#[test]
	fn test_parse_simple_jsx_element() {
		let input = "<div className=\"container\">Hello, world!</div>";
		let lexer = Lexer::new(input);
		let mut parser = Parser::new(lexer);

		let result = parser.parse().unwrap();
		assert_eq!(result.len(), 1);

		if let Node::Element(element) = &result[0] {
			assert_eq!(element.tag_name, "div");
			assert_eq!(element.attributes.len(), 1);
			assert_eq!(element.attributes[0].name, "className");

			if let Some(AttributeValue::String(value)) = &element.attributes[0].value {
				assert_eq!(value, "container");
			} else {
				panic!("Expected string attribute value");
			}

			assert_eq!(element.children.len(), 1);

			if let Node::Text(text) = &element.children[0] {
				assert_eq!(text, "Hello, world!");
			} else {
				panic!("Expected text node");
			}
		} else {
			panic!("Expected element node");
		}
	}

	#[test]
	fn test_parse_jsx_with_expression() {
		let input = "<button onClick={() => alert('Hello')}>Click me</button>";
		let lexer = Lexer::new(input);
		let mut parser = Parser::new(lexer);

		let result = parser.parse().unwrap();
		assert_eq!(result.len(), 1);

		if let Node::Element(element) = &result[0] {
			assert_eq!(element.tag_name, "button");
			assert_eq!(element.attributes.len(), 1);
			assert_eq!(element.attributes[0].name, "onClick");

			if let Some(AttributeValue::Expression(expr)) = &element.attributes[0].value {
				if let JSXExpression::ArrowFunctionExpression { parameters, body: _ } = expr {
					assert_eq!(parameters.len(), 0);
				} else {
					panic!("Expected arrow function expression");
				}
			} else {
				panic!("Expected expression attribute value");
			}

			assert_eq!(element.children.len(), 1);

			if let Node::Text(text) = &element.children[0] {
				assert_eq!(text, "Click me");
			} else {
				panic!("Expected text node");
			}
		} else {
			panic!("Expected element node");
		}
	}
}
