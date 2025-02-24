use std::path::Path;
use std::sync::Arc;
use swc_core::{
	common::{errors::Handler, sync::Lrc, FileName, SourceMap},
	ecma::parser::{Parser, StringInput, Syntax, TsConfig},
	ecma::{
		ast::*,
		visit::{Visit, VisitWith},
	},
};

mod ast_utils;
use ast_utils::*;

struct ClassNameCollector {
	class_names: Vec<String>,
}

impl ClassNameCollector {
	fn new() -> Self {
		Self { class_names: Vec::new() }
	}
}

impl Visit for ClassNameCollector {
	fn visit_jsx_element(&mut self, jsx: &JSXElement) {
		// Visit JSX attributes to find className
		for attr in &jsx.opening.attrs {
			if let JSXAttrOrSpread::JSXAttr(jsx_attr) = attr {
				// Check if it's a class attribute
				if is_class_attribute(jsx_attr, r"^className$") {
					if let Some(class_value) = extract_value_from_node(jsx_attr) {
						let extraction = extract_classnames_from_value(Some(class_value));
						self.class_names.extend(extraction.class_names);
					}
				}
			}
		}

		// Continue visiting child elements
		jsx.children.iter().for_each(|child| child.visit_with(self));
	}
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	// Example React component code
	let source_code = r#"
            import React from 'react';
                
                    const ExampleComponent = () => {
                          return (
                                  <div className="container mx-auto p-4">
                                            <h1 className="text-2xl font-bold mb-4">Hello World</h1>
                                                      <button className="bg-blue-500 hover:bg-blue-700 text-white font-bold py-2 px-4 rounded">
                                                                  Click me
                                                                            </button>
                                                                                    </div>
                                                                                          );
                                                                                              };
                                                                                                  
                                                                                                      export default ExampleComponent;
                                                                                                          "#;

	// Set up the parser
	let source_map: Lrc<SourceMap> = Default::default();
	let source_file = source_map.new_source_file(FileName::Custom("example.jsx".into()), source_code.into());

	let handler = Handler::with_tty_emitter(source_map.clone(), false, false);

	let syntax = Syntax::Es(Default::default());
	let mut parser = Parser::new(syntax, StringInput::from(&source_file), None);

	let module = parser.parse_module().map_err(|e| {
		e.into_diagnostic(&handler).emit();
		std::io::Error::new(std::io::ErrorKind::Other, "Failed to parse module")
	})?;

	// Collect classnames using our visitor
	let mut collector = ClassNameCollector::new();
	module.visit_with(&mut collector);

	println!("Found the following Tailwind classes:");
	let unique_classes = remove_duplicates_from_array(collector.class_names);
	for class_name in unique_classes {
		println!("  - {}", class_name);
	}

	// Example of using the parseNodeRecursive function
	println!("\nAdditional Examples:");

	// Create a mock JSX attribute with classes
	let expr = create_jsx_class_attribute("btn btn-primary mx-2");

	// Define a callback to process classes
	let class_callback = |classes: Vec<String>, _node: Option<&Expr>| {
		println!("Processing classes via callback:");
		for class in classes {
			println!("  - {}", class);
		}
	};

	// Parse the node recursively
	parse_node_recursive(Some(&expr), None, class_callback, false, false, &[]);

	Ok(())
}

// Helper function to create a JSX class attribute expression for examples
fn create_jsx_class_attribute(class_value: &str) -> Expr {
	Expr::Lit(Lit::Str(Str {
		span: swc_core::common::DUMMY_SP,
		value: class_value.into(),
		has_escape: false,
	}))
}
