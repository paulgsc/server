use regex::Regex;
use std::collections::HashSet;
use swc_core::common::Spanned;
use swc_ecma_ast::{BinaryOp, Expr, JSXAttr, JSXAttrName, JSXAttrValue, JSXExpr, Lit, MemberProp, Prop, PropName, PropOrSpread, Str};

/// Utility functions for working with Abstract Syntax Trees (AST)
/// Ported from JavaScript to Rust using SWC

/// Convert a callee node to a string representation
pub fn callee_to_string(callee: &Expr) -> Option<String> {
	match callee {
		Expr::Ident(id) => Some(id.sym.to_string()),
		Expr::Member(member) => {
			if let (Expr::Ident(obj), MemberProp::Ident(prop)) = (&*member.obj, &member.prop) {
				Some(format!("{}.{}", obj.sym, prop.sym))
			} else {
				None
			}
		}
		_ => None,
	}
}

/// Check if a node represents a class attribute
pub fn is_class_attribute(node: &JSXAttr, class_regex: &str) -> bool {
	let name = match &node.name {
		JSXAttrName::Ident(ident) => ident.sym.to_string(),
		JSXAttrName::JSXNamespacedName(ns) => {
			let namespace = ns.ns.sym.to_string();
			let name = ns.name.sym.to_string();
			if namespace.is_empty() {
				name
			} else {
				format!("{}:{}", namespace, name)
			}
		}
	};

	Regex::new(class_regex).unwrap().is_match(&name)
}

/// Check if a node's value is a literal attribute value
pub fn is_literal_attribute_value(node: &JSXAttr) -> bool {
	if let Some(value) = &node.value {
		match value {
			JSXAttrValue::Lit(lit) => {
				if let Lit::Str(str_lit) = lit {
					// No support for dynamic or conditional expressions
					!Regex::new(r"\{|\?|\}").unwrap().is_match(&str_lit.value.to_string())
				} else {
					false
				}
			}
			JSXAttrValue::JSXExprContainer(container) => {
				matches!(
					&container.expr,
					JSXExpr::Expr(expr) if matches!(expr.as_ref(), Expr::Lit(_))
				)
			}
			_ => false,
		}
	} else {
		false
	}
}

/// Check if a node is a valid JSX attribute for our rules
pub fn is_valid_jsx_attribute(node: &JSXAttr, class_regex: &str) -> bool {
	if !is_class_attribute(node, class_regex) {
		// Only run for class[Name] attributes
		return false;
	}

	if !is_literal_attribute_value(node) {
		// No support for dynamic or conditional classnames
		return false;
	}

	true
}

/// Check if a node represents an array expression
pub fn is_array_expression(expr: &Expr) -> bool {
	matches!(expr, Expr::Array(_))
}

/// Check if a node represents an object expression
pub fn is_object_expression(expr: &Expr) -> bool {
	matches!(expr, Expr::Object(_))
}

/// Extract range (start and end positions) from a node
pub fn extract_range_from_node(node: &JSXAttr) -> (u32, u32) {
	if let Some(value) = &node.value {
		match value {
			JSXAttrValue::JSXExprContainer(container) => {
				if let JSXExpr::Expr(expr) = &container.expr {
					let span = expr.span();
					(span.lo.0, span.hi.0)
				} else {
					(0, 0)
				}
			}
			JSXAttrValue::Lit(lit) => {
				let span = lit.span();
				(span.lo.0, span.hi.0)
			}
			_ => (0, 0),
		}
	} else {
		(0, 0)
	}
}

/// Extract string value from a node
pub fn extract_value_from_node(node: &JSXAttr) -> Option<String> {
	if let Some(value) = &node.value {
		match value {
			JSXAttrValue::Lit(lit) => {
				if let Lit::Str(str_lit) = lit {
					Some(str_lit.value.to_string())
				} else {
					None
				}
			}
			JSXAttrValue::JSXExprContainer(container) => {
				if let JSXExpr::Expr(Expr::Lit(Lit::Str(str_lit))) = &container.expr {
					Some(str_lit.value.to_string())
				} else {
					None
				}
			}
			_ => None,
		}
	} else {
		None
	}
}

/// Class extraction result with metadata
pub struct ClassExtractionResult {
	pub class_names: Vec<String>,
	pub whitespaces: Vec<String>,
	pub head_space: bool,
	pub tail_space: bool,
}

/// Extract classnames from a string value
pub fn extract_classnames_from_value(class_str: Option<String>) -> ClassExtractionResult {
	if class_str.is_none() {
		return ClassExtractionResult {
			class_names: vec![],
			whitespaces: vec![],
			head_space: false,
			tail_space: false,
		};
	}

	let class_str = class_str.unwrap();
	let separator_regex = Regex::new(r"\s+").unwrap();

	let mut parts: Vec<String> = separator_regex.split(&class_str).map(|s| s.to_string()).collect();

	if parts.first().map_or(false, |s| s.is_empty()) {
		parts.remove(0);
	}

	if parts.last().map_or(false, |s| s.is_empty()) {
		parts.pop();
	}

	let head_space = separator_regex.is_match(parts.first().unwrap_or(&String::new()));
	let tail_space = separator_regex.is_match(parts.last().unwrap_or(&String::new()));

	let (class_names, whitespaces): (Vec<String>, Vec<String>) = parts.into_iter().enumerate().partition(|(i, _)| if head_space { i % 2 != 0 } else { i % 2 == 0 });

	let class_names = class_names.into_iter().map(|(_, s)| s).collect();
	let whitespaces = whitespaces.into_iter().map(|(_, s)| s).collect();

	ClassExtractionResult {
		class_names,
		whitespaces,
		head_space,
		tail_space,
	}
}

/// Remove duplicates from a vector while preserving order
pub fn remove_duplicates_from_array<T: Clone + Eq + std::hash::Hash>(arr: Vec<T>) -> Vec<T> {
	let mut seen = HashSet::new();
	let mut result = Vec::new();

	for item in arr {
		if seen.insert(item.clone()) {
			result.push(item);
		}
	}

	result
}

/// Type definition for callback functions
pub type NodeCallback = fn(Vec<String>, Option<&Expr>);

/// Parse a node recursively and run a callback function
pub fn parse_node_recursive(root_node: Option<&Expr>, child_node: Option<&Expr>, callback: NodeCallback, skip_conditional: bool, isolate: bool, ignored_keys: &[String]) {
	if child_node.is_none() {
		// Process root node
		if let Some(root) = root_node {
			let original_classnames_value = match root {
				Expr::JSXElement(jsx) => {
					// Extract from JSX props
					None // Simplified - would need to find class prop
				}
				Expr::Lit(Lit::Str(str_lit)) => Some(str_lit.value.to_string()),
				_ => None,
			};

			let extraction_result = extract_classnames_from_value(original_classnames_value);
			let mut class_names = extraction_result.class_names;

			class_names = remove_duplicates_from_array(class_names);

			if !class_names.is_empty() {
				callback(class_names, root_node);
			}
		}
		return;
	}

	let child = child_node.unwrap();
	let force_isolation = if skip_conditional { true } else { isolate };

	match child {
		Expr::Tpl(template) => {
			// Process template expressions
			for expr in &template.exprs {
				parse_node_recursive(root_node, Some(expr), callback, skip_conditional, force_isolation, ignored_keys);
			}

			// Process template quasis
			for quasi in &template.quasis {
				if let Some(raw) = &quasi.raw {
					let expr = Expr::Lit(Lit::Str(Str {
						span: quasi.span,
						value: raw.clone(),
						has_escape: false,
					}));
					parse_node_recursive(root_node, Some(&expr), callback, skip_conditional, isolate, ignored_keys);
				}
			}
		}
		Expr::Cond(cond) => {
			// Process conditional expressions
			parse_node_recursive(root_node, Some(&cond.cons), callback, skip_conditional, force_isolation, ignored_keys);
			parse_node_recursive(root_node, Some(&cond.alt), callback, skip_conditional, force_isolation, ignored_keys);
		}
		Expr::Bin(bin) => {
			// Process logical expressions
			if matches!(bin.op, BinaryOp::LogicalOr | BinaryOp::LogicalAnd) {
				parse_node_recursive(root_node, Some(&bin.right), callback, skip_conditional, force_isolation, ignored_keys);
			}
		}
		Expr::Array(array) => {
			// Process array elements
			for elem in &array.elems {
				if let Some(expr) = elem.as_ref().map(|e| &*e.expr) {
					parse_node_recursive(root_node, Some(expr), callback, skip_conditional, force_isolation, ignored_keys);
				}
			}
		}
		Expr::Object(obj) => {
			// Process object properties
			let is_used_by_classnames_plugin = if let Some(root) = root_node {
				match root {
					Expr::Call(call) => callee_to_string(&call.callee).map_or(false, |name| name == "classnames"),
					_ => false,
				}
			} else {
				false
			};

			for prop in &obj.props {
				match prop {
					PropOrSpread::Spread(_) => {
						// Ignore spread elements
						continue;
					}
					PropOrSpread::Prop(prop) => {
						if let Prop::KeyValue(kv) = &**prop {
							if let PropName::Ident(id) = &kv.key {
								if ignored_keys.contains(&id.sym.to_string()) {
									// Ignore specific keys defined in settings
									continue;
								}
							}

							parse_node_recursive(
								root_node,
								Some(if is_used_by_classnames_plugin {
									match &kv.key {
										PropName::Ident(id) => &Expr::Lit(Lit::Str(Str {
											span: id.span,
											value: id.sym.clone(),
											has_escape: false,
										})),
										_ => &kv.value,
									}
								} else {
									&kv.value
								}),
								callback,
								skip_conditional,
								force_isolation,
								ignored_keys,
							);
						}
					}
				}
			}
		}
		Expr::Lit(Lit::Str(str_lit)) => {
			// Process literal values
			let original_classnames_value = Some(str_lit.value.to_string());
			let extraction_result = extract_classnames_from_value(original_classnames_value);
			let mut class_names = extraction_result.class_names;

			class_names = remove_duplicates_from_array(class_names);

			if !class_names.is_empty() {
				let target_node = if isolate { None } else { root_node };
				callback(class_names, target_node);
			}
		}
		_ => {
			// Other expression types not handled specifically
		}
	}
}

/// Get the prefix of a template element
pub fn get_template_element_prefix(text: &str, raw: &str) -> String {
	if let Some(idx) = text.find(raw) {
		if idx == 0 {
			String::new()
		} else {
			text.split(raw).next().unwrap_or("").to_string()
		}
	} else {
		String::new()
	}
}

/// Get the suffix of a template element
pub fn get_template_element_suffix(text: &str, raw: &str) -> String {
	if text.contains(raw) {
		text.split(raw).last().unwrap_or("").to_string()
	} else {
		String::new()
	}
}

/// Get the body content of a template element
pub fn get_template_element_body(text: &str, prefix: &str, suffix: &str) -> String {
	let mut parts: Vec<&str> = text.split(prefix).collect();
	if !parts.is_empty() {
		parts.remove(0);
	}

	let body = parts.join(prefix);
	let mut parts: Vec<&str> = body.split(suffix).collect();
	if !parts.is_empty() {
		parts.pop();
	}

	parts.join(suffix)
}

#[cfg(test)]
mod tests {
	use super::*;
	use swc_core::common::{BytePos, Span, DUMMY_SP};
	use swc_core::ecma::ast::*;

	fn create_string_literal(value: &str) -> Box<Expr> {
		Box::new(Expr::Lit(Lit::Str(Str {
			span: DUMMY_SP,
			value: value.into(),
			has_escape: false,
		})))
	}

	fn create_jsx_attr(name: &str, value: Option<&str>) -> JsxAttr {
		let name = JsxAttrName::Ident(Ident {
			span: DUMMY_SP,
			sym: name.into(),
			optional: false,
		});

		let value = value.map(|v| {
			JsxAttrValue::Lit(Lit::Str(Str {
				span: DUMMY_SP,
				value: v.into(),
				has_escape: false,
			}))
		});

		JsxAttr { span: DUMMY_SP, name, value }
	}

	#[test]
	fn test_callee_to_string() {
		// Test Identifier
		let ident_expr = Expr::Ident(Ident {
			span: DUMMY_SP,
			sym: "testFunction".into(),
			optional: false,
		});
		assert_eq!(callee_to_string(&ident_expr), Some("testFunction".to_string()));

		// Test MemberExpression
		let member_expr = Expr::Member(MemberExpr {
			span: DUMMY_SP,
			obj: Box::new(Expr::Ident(Ident {
				span: DUMMY_SP,
				sym: "React".into(),
				optional: false,
			})),
			prop: MemberProp::Ident(Ident {
				span: DUMMY_SP,
				sym: "createElement".into(),
				optional: false,
			}),
		});
		assert_eq!(callee_to_string(&member_expr), Some("React.createElement".to_string()));
	}

	#[test]
	fn test_is_class_attribute() {
		let class_attr = create_jsx_attr("className", Some("my-class"));
		let style_attr = create_jsx_attr("style", Some("color: red"));

		assert!(is_class_attribute(&class_attr, r"^className$"));
		assert!(!is_class_attribute(&style_attr, r"^className$"));
		assert!(is_class_attribute(&class_attr, r"^class|className$"));
	}

	#[test]
	fn test_is_literal_attribute_value() {
		let class_attr = create_jsx_attr("className", Some("my-class"));
		assert!(is_literal_attribute_value(&class_attr));

		// Test with no value
		let empty_attr = create_jsx_attr("className", None);
		assert!(!is_literal_attribute_value(&empty_attr));
	}

	#[test]
	fn test_is_valid_jsx_attribute() {
		let class_attr = create_jsx_attr("className", Some("my-class"));
		let style_attr = create_jsx_attr("style", Some("color: red"));
		let empty_attr = create_jsx_attr("className", None);

		assert!(is_valid_jsx_attribute(&class_attr, r"^className$"));
		assert!(!is_valid_jsx_attribute(&style_attr, r"^className$"));
		assert!(!is_valid_jsx_attribute(&empty_attr, r"^className$"));
	}

	#[test]
	fn test_extract_classnames_from_value() {
		// Basic test with multiple classes
		let result = extract_classnames_from_value(Some("btn btn-primary mr-2".to_string()));
		assert_eq!(result.class_names, vec!["btn", "btn-primary", "mr-2"]);
		assert!(!result.head_space);

		// Test with leading and trailing spaces
		let result = extract_classnames_from_value(Some("  btn  btn-primary  ".to_string()));
		assert!(result.head_space);
		assert!(result.tail_space);
		assert_eq!(result.class_names, vec!["btn", "btn-primary"]);

		// Test with None
		let result = extract_classnames_from_value(None);
		assert!(result.class_names.is_empty());
	}

	#[test]
	fn test_remove_duplicates_from_array() {
		let input = vec!["btn".to_string(), "primary".to_string(), "btn".to_string(), "large".to_string()];
		let result = remove_duplicates_from_array(input);
		assert_eq!(result, vec!["btn".to_string(), "primary".to_string(), "large".to_string()]);
	}

	#[test]
	fn test_template_element_helpers() {
		let text = "Hello ${name} world";
		let raw = "${name}";

		let prefix = get_template_element_prefix(text, raw);
		assert_eq!(prefix, "Hello ");

		let suffix = get_template_element_suffix(text, raw);
		assert_eq!(suffix, " world");

		let body = get_template_element_body(text, "Hello ", " world");
		assert_eq!(body, "${name}");
	}
}
