use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, Data, DeriveInput, Error, Field, Fields, Ident, Lit, Meta, MetaNameValue, NestedMeta, Result};

/// Where a struct field reads from, as declared by its `#[gsheet(...)]`
/// attribute.
struct ColumnBinding {
	column: String,
	required: bool,
}

#[proc_macro_derive(FromGSheet, attributes(gsheet))]
pub fn from_gsheet_derive(input: TokenStream) -> TokenStream {
	let input = parse_macro_input!(input as DeriveInput);
	// Every rejection is reported as a `compile_error!` at the offending span
	// rather than a macro panic, so the caller sees which field is wrong
	// instead of a backtrace with the whole derive aborted.
	expand(&input).unwrap_or_else(Error::into_compile_error).into()
}

fn expand(input: &DeriveInput) -> Result<TokenStream2> {
	let name = &input.ident;
	let fields = named_fields(input)?;

	let mut column_mappings = Vec::with_capacity(fields.len());
	let mut field_parsers = Vec::with_capacity(fields.len());
	let mut field_names = Vec::with_capacity(fields.len());

	for field in fields {
		let field_ident = field_ident(field)?;
		let field_name = field_ident.to_string();
		let binding = column_binding(field)?;
		let column = &binding.column;
		let required = binding.required;

		column_mappings.push(quote! { (#field_name.to_string(), #column.to_string(), #required) });
		field_parsers.push(field_parser(field, field_ident, &field_name, &binding));
		field_names.push(field_ident);
	}

	Ok(quote! {
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
	})
}

/// The named fields of the derived-on struct, or an error naming the shape
/// that was found instead.
fn named_fields(input: &DeriveInput) -> Result<&syn::punctuated::Punctuated<Field, syn::Token![,]>> {
	match &input.data {
		Data::Struct(data) => match &data.fields {
			Fields::Named(fields) => Ok(&fields.named),
			other => Err(Error::new_spanned(other, "FromGSheet derive only works on structs with named fields")),
		},
		Data::Enum(_) | Data::Union(_) => Err(Error::new_spanned(input, "FromGSheet derive only works on structs")),
	}
}

/// The identifier of a named field.
///
/// `named_fields` has already established that these are named, so the `None`
/// arm is unreachable in practice — it is reported rather than unwrapped so a
/// future caller cannot turn it into a panic.
fn field_ident(field: &Field) -> Result<&Ident> {
	field
		.ident
		.as_ref()
		.ok_or_else(|| Error::new_spanned(field, "FromGSheet derive only works on structs with named fields"))
}

/// Read `#[gsheet(column = "X", required)]` off a field.
///
/// Both halves of the expansion — the column mapping and the row parser — need
/// the same answer, so it is parsed once here rather than twice inline.
fn column_binding(field: &Field) -> Result<ColumnBinding> {
	let mut column = String::new();
	let mut required = false;

	for attr in &field.attrs {
		if !attr.path.is_ident("gsheet") {
			continue;
		}
		let Ok(Meta::List(meta_list)) = attr.parse_meta() else {
			continue;
		};
		for nested in &meta_list.nested {
			match nested {
				NestedMeta::Meta(Meta::NameValue(MetaNameValue { path, lit: Lit::Str(lit_str), .. })) if path.is_ident("column") => {
					column = lit_str.value();
				}
				NestedMeta::Meta(Meta::Path(path)) if path.is_ident("required") => {
					required = true;
				}
				_ => {}
			}
		}
	}

	if column.is_empty() {
		return Err(Error::new_spanned(field, "missing #[gsheet(column = \"X\")] attribute"));
	}

	Ok(ColumnBinding { column, required })
}

/// The `let #field_name = ...;` statement that pulls one field out of a row.
///
/// `Option<T>` fields absorb a missing or blank cell as `None`; required
/// fields error on a missing cell; the rest fall back to the type's default.
fn field_parser(field: &Field, field_ident: &Ident, field_name: &str, binding: &ColumnBinding) -> TokenStream2 {
	let column = &binding.column;
	let required = binding.required;
	let is_option = field.ty.to_token_stream().to_string().starts_with("Option <");

	if is_option {
		quote! {
			let #field_ident = match get_cell_value(row, #column, header_map, #field_name, #required)? {
				Some(val) if !val.is_empty() => Some(parse_cell(val, #field_name, #column)?),
				_ => None,
			};
		}
	} else if required {
		quote! {
			let #field_ident = get_cell_value(row, #column, header_map, #field_name, #required)?
				.ok_or_else(|| GSheetDeriveError::MissingRequiredField(#field_name.to_string(), #column.to_string()))?;
			let #field_ident = parse_cell(#field_ident, #field_name, #column)?;
		}
	} else {
		quote! {
			let #field_ident = get_cell_value(row, #column, header_map, #field_name, #required)?
				.unwrap_or_default();
			let #field_ident = parse_cell(#field_ident, #field_name, #column)?;
		}
	}
}
