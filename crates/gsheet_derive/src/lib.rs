use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, Data, DeriveInput, Fields, Lit, Meta, MetaNameValue, NestedMeta};

#[proc_macro_derive(FromGSheet, attributes(gsheet))]
pub fn from_gsheet_derive(input: TokenStream) -> TokenStream {
	let input = parse_macro_input!(input as DeriveInput);
	let name = input.ident;

	// Extract struct fields
	let fields = match input.data {
		Data::Struct(data) => match data.fields {
			Fields::Named(fields) => fields.named,
			_ => panic!("FromGSheet derive only works on structs with named fields"),
		},
		_ => panic!("FromGSheet derive only works on structs"),
	};

	// Generate column mapping
	let column_mappings = fields.iter().map(|field| {
		let field_name = field.ident.as_ref().unwrap().to_string();
		let mut column = "".to_string();
		let mut required = false;

		for attr in &field.attrs {
			if attr.path.is_ident("gsheet") {
				if let Ok(Meta::List(meta_list)) = attr.parse_meta() {
					for nested in meta_list.nested.iter() {
						match nested {
							NestedMeta::Meta(Meta::NameValue(MetaNameValue { path, lit, .. })) => {
								if path.is_ident("column") {
									if let Lit::Str(lit_str) = lit {
										column = lit_str.value();
									}
								}
							}
							NestedMeta::Meta(Meta::Path(path)) => {
								if path.is_ident("required") {
									required = true;
								}
							}
							_ => {}
						}
					}
				}
			}
		}

		if column.is_empty() {
			panic!("Field {} missing #[gsheet(column = \"X\")] attribute", field_name);
		}

		quote! {
		(#field_name.to_string(), #column.to_string(), #required)
				}
	});

	// Generate field parsing
	let field_parsers = fields.iter().map(|field| {
		let field_name = field.ident.as_ref().unwrap();
		let field_name_str = field_name.to_string();
		let field_type = &field.ty;

		let mut column = "".to_string();
		let mut required = false;

		for attr in &field.attrs {
			if attr.path.is_ident("gsheet") {
				if let Ok(Meta::List(meta_list)) = attr.parse_meta() {
					for nested in meta_list.nested.iter() {
						match nested {
							NestedMeta::Meta(Meta::NameValue(MetaNameValue { path, lit, .. })) => {
								if path.is_ident("column") {
									if let Lit::Str(lit_str) = lit {
										column = lit_str.value();
									}
								}
							}
							NestedMeta::Meta(Meta::Path(path)) => {
								if path.is_ident("required") {
									required = true;
								}
							}
							_ => {}
						}
					}
				}
			}
		}

		// Check if it's an Option type
		let is_option = match field_type.to_token_stream().to_string().as_str() {
			s if s.starts_with("Option < ") => true,
			_ => false,
		};

		if is_option {
			quote! {
				let #field_name = match get_cell_value(row, #column, header_map, #field_name_str, #required)? {
					Some(val) if !val.is_empty() => Some(parse_cell(val, #field_name_str, #column)?),
					_ => None,
				};
			}
		} else if required {
			quote! {
				let #field_name = get_cell_value(row, #column, header_map, #field_name_str, #required)?
					.ok_or_else(|| GSheetDeriveError::MissingRequiredField(#field_name_str.to_string(), #column.to_string()))?;
				let #field_name = parse_cell(#field_name, #field_name_str, #column)?;
			}
		} else {
			quote! {
				let #field_name = get_cell_value(row, #column, header_map, #field_name_str, #required)?
					.unwrap_or_default();
				let #field_name = parse_cell(#field_name, #field_name_str, #column)?;
			}
		}
	});

	// Generate struct construction
	let field_names = fields.iter().map(|field| {
		let field_name = field.ident.as_ref().unwrap();
		quote! { #field_name }
	});

	// Generate the impl
	let expanded = quote! {
	impl FromGSheet for #name {
		fn column_mapping() -> Vec<(String, String, bool)> {
			vec![
				#(#column_mappings),*
			]
		}

		fn from_gsheet_row(row: &[String], header_map: &HashMap<String, usize>) -> Result<Self, GSheetDeriveError> {
			#(#field_parsers)*

			Ok(Self {
				#(#field_names),*
			})
		}
	}
	};

	TokenStream::from(expanded)
}
