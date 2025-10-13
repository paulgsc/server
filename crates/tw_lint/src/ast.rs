use std::fmt;
/// Represents a node in the Abstract Syntax Tree (AST).
#[derive(Debug, PartialEq, Clone)]
pub enum Node {
	/// Represents a JSX element, such as `<div class="container">Hello</div>`.
	Element(Element),
	/// Represents a text node within a JSX element.
	Text(String),
	/// Represents a JSX expression, such as `{variable}`.
	JSXExpression(JSXExpression),
	/// Represents a fragment, such as `<></>`.
	Fragment(Fragment),
	/// Represents a comment.
	Comment(String),
}

/// Represents a JSX element.
#[derive(Debug, PartialEq, Clone)]
pub struct Element {
	/// The name of the element (e.g., "div", "MyComponent").
	pub tag_name: String, // Changed from 'name' to 'tag_name' to be consistent
	/// The attributes of the element.
	pub attributes: Vec<JSXAttribute>,
	/// The children of the element.
	pub children: Vec<Node>,
	/// Is this a self-closing tag?
	pub self_closing: bool, // Changed from is_self_closing to self_closing
	/// Optional TypeScript generic type arguments
	pub type_arguments: Option<String>,
}

/// Represents a JSX fragment.
#[derive(Debug, PartialEq, Clone)]
pub struct Fragment {
	/// The children of the fragment.
	pub children: Vec<Node>,
}

/// Represents a JSX attribute.
#[derive(Debug, PartialEq, Clone)]
pub struct JSXAttribute {
	/// The name of the attribute.
	pub name: String,
	/// The value of the attribute, which can be a string or an expression.
	pub value: Option<AttributeValue>, // Corrected Option name
}

/// Possible values for a JSX attribute
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeValue {
	/// A string literal value.
	String(String),
	/// An expression value.
	Expression(JSXExpression),
}

/// Represents a JSX expression.  This is the content inside curly braces.
#[derive(Debug, PartialEq, Clone)]
pub enum JSXExpression {
	/// An identifier (e.g., `variable`, `handleClick`).
	Identifier(String),
	/// A string literal (e.g., `"hello"`, `'world'`).
	StringLiteral(String),
	/// A number literal (e.g., `123`, `3.14`).
	NumberLiteral(String), // Use String to keep the exact representation from the source
	/// A boolean literal (e.g., `true`, `false`).
	BooleanLiteral(bool),
	/// A null literal.
	NullLiteral,
	/// An undefined literal.
	UndefinedLiteral,
	/// An object expression (e.g., `{ color: 'red', fontSize: 16 }`).
	ObjectExpression(Vec<(String, Box<JSXExpression>)>),
	/// An array expression (e.g., `[1, 2, 3]`).
	ArrayExpression(Vec<Box<JSXExpression>>),
	/// A function call (e.g., `handleClick()`, `formatName(firstName, lastName)`).
	CallExpression { name: String, arguments: Vec<JSXExpression> },
	/// A member expression (e.g., `props.name`, `obj.property.nested`).
	MemberExpression { object: Box<JSXExpression>, property: String },
	/// A binary expression (e.g., `1 + 2`, `a === b`).
	BinaryExpression {
		operator: String,
		left: Box<JSXExpression>,
		right: Box<JSXExpression>,
	},
	/// A unary expression (e.g., `-x`, `!flag`).
	UnaryExpression { operator: String, argument: Box<JSXExpression> },
	/// A conditional expression (e.g., `condition ? trueValue : falseValue`).
	ConditionalExpression {
		condition: Box<JSXExpression>,
		consequent: Box<JSXExpression>,
		alternate: Box<JSXExpression>,
	},
	/// An arrow function expression (e.g., `() => {}`, `(x) => x * 2`).
	ArrowFunctionExpression {
		parameters: Vec<String>,  // Simplified parameter list.
		body: Box<JSXExpression>, //  For simplicity, the body is an expression
	},
	/// A block of code (e.g., `{ let x = 1; return x; }`).  Used for arrow function bodies.
	BlockStatement(Vec<Node>),
	/// A return statement.
	ReturnStatement(Box<JSXExpression>),
	/// An element
	JSXElement(Element), //nested JSX element
}

impl fmt::Display for JSXExpression {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Identifier(name) => write!(f, "{}", name),
			Self::StringLiteral(s) => write!(f, "\"{}\"", s),
			Self::NumberLiteral(n) => write!(f, "{}", n),
			Self::BooleanLiteral(b) => write!(f, "{}", b),
			Self::NullLiteral => write!(f, "null"),
			Self::UndefinedLiteral => write!(f, "undefined"),
			Self::ObjectExpression(props) => {
				write!(f, "{{ ")?;
				for (i, (key, value)) in props.iter().enumerate() {
					write!(f, "{}: {}", key, value)?;
					if i < props.len() - 1 {
						write!(f, ", ")?;
					}
				}
				write!(f, " }}")
			}
			Self::ArrayExpression(elements) => {
				write!(f, "[")?;
				for (i, element) in elements.iter().enumerate() {
					write!(f, "{}", element)?;
					if i < elements.len() - 1 {
						write!(f, ", ")?;
					}
				}
				write!(f, "]")
			}
			Self::CallExpression { name, arguments } => {
				write!(f, "{}(", name)?;
				for (i, arg) in arguments.iter().enumerate() {
					write!(f, "{}", arg)?;
					if i < arguments.len() - 1 {
						write!(f, ", ")?;
					}
				}
				write!(f, ")")
			}
			Self::MemberExpression { object, property } => {
				write!(f, "{}.{}", object, property)
			}
			Self::BinaryExpression { operator, left, right } => write!(f, "{} {} {}", left, operator, right),
			Self::UnaryExpression { operator, argument } => {
				write!(f, "{}{}", operator, argument)
			}
			Self::ConditionalExpression { condition, consequent, alternate } => write!(f, "{} ? {} : {}", condition, consequent, alternate),
			Self::ArrowFunctionExpression { parameters, body } => {
				write!(f, "(")?;
				for (i, param) in parameters.iter().enumerate() {
					write!(f, "{}", param)?;
					if i < parameters.len() - 1 {
						write!(f, ", ")?;
					}
				}
				write!(f, ") => {}", body)
			}
			Self::BlockStatement(nodes) => {
				write!(f, "{{ ")?;
				for node in nodes {
					write!(f, "{node:?}")?; // Use Debug for Node
				}
				write!(f, " }}")
			}
			Self::ReturnStatement(expr) => {
				write!(f, "return {expr}")
			}
			Self::JSXElement(el) => {
				write!(f, "{el:?}",)
			}
		}
	}
}

impl Element {
	/// Create a new element with the given tag name
	pub fn new(tag_name: &str) -> Self {
		Self {
			tag_name: tag_name.to_string(),
			attributes: Vec::new(),
			children: Vec::new(),
			self_closing: false,
			type_arguments: None,
		}
	}

	/// Add an attribute to the element
	pub fn add_attribute(&mut self, name: &str, value: Option<AttributeValue>) {
		self.attributes.push(JSXAttribute { name: name.to_string(), value });
	}

	/// Add a child node to the element
	pub fn add_child(&mut self, child: Node) {
		self.children.push(child);
	}

	/// Set whether the element is self-closing
	pub fn set_self_closing(&mut self, self_closing: bool) {
		self.self_closing = self_closing;
	}

	/// Set TypeScript generic type arguments
	pub fn set_type_arguments(&mut self, type_args: &str) {
		self.type_arguments = Some(type_args.to_string());
	}
}
