use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Attribute, Data, DeriveInput, Fields, Type};

// ---------------------- Entity Derive ----------------------

#[proc_macro_derive(Entity, attributes(entity, primary_key, table_name))]
pub fn derive_entity(input: TokenStream) -> TokenStream {
	let input = parse_macro_input!(input as DeriveInput);
	let name = &input.ident;

	let table_name = get_table_name(&input.attrs, &name.to_string());
	let (pk_field, pk_type) = get_primary_key_info(&input);

	let fields: Vec<_> = if let Data::Struct(data_struct) = &input.data {
		if let Fields::Named(fields_named) = &data_struct.fields {
			fields_named.named.iter().collect()
		} else {
			panic!("Entity derive only supports named fields");
		}
	} else {
		panic!("Entity derive only supports structs");
	};

	let column_names: Vec<String> = fields.iter().map(|f| f.ident.as_ref().unwrap().to_string()).collect();

	let value_exprs: Vec<proc_macro2::TokenStream> = fields
		.iter()
		.map(|f| {
			let ident = &f.ident;
			let ty = &f.ty;

			if is_uuid_type(ty) {
				quote! { some_sqlite::QueryValue::String(self.#ident.to_string()) }
			} else if is_option_type(ty) {
				quote! { self.#ident.clone().map(|v| v.into()).unwrap_or(some_sqlite::QueryValue::Null) }
			} else {
				quote! { self.#ident.clone().into() }
			}
		})
		.collect();

	let expanded = quote! {
		impl some_sqlite::Entity for #name {
			type Id = #pk_type;

			fn id(&self) -> &Self::Id {
				&self.#pk_field
			}

			fn table_name() -> &'static str {
				#table_name
			}

			fn pk_column() -> &'static str {
				stringify!(#pk_field)
			}

			fn columns_and_values(&self) -> (Vec<&str>, Vec<some_sqlite::QueryValue>) {
				let columns: Vec<&str> = vec![#(#column_names),*];
				let values: Vec<some_sqlite::QueryValue> = vec![#(#value_exprs),*];
				(columns, values)
			}
		}
	};

	TokenStream::from(expanded)
}

// ---------------------- NewEntity Derive ----------------------

#[proc_macro_derive(NewEntity, attributes(new_entity, table_name))]
pub fn derive_new_entity(input: TokenStream) -> TokenStream {
	let input = parse_macro_input!(input as DeriveInput);
	let name = &input.ident;

	// Parse #[new_entity(entity = "User")]
	let entity_type = get_entity_attr(&input.attrs);

	let fields = if let Data::Struct(data_struct) = &input.data {
		if let Fields::Named(fields_named) = &data_struct.fields {
			fields_named.named.iter().collect::<Vec<_>>()
		} else {
			panic!("NewEntity derive only supports named fields");
		}
	} else {
		panic!("NewEntity derive only supports structs");
	};

	// Columns: skip "id" if it's UUID
	let column_names: Vec<String> = fields
		.iter()
		.filter(|f| !(is_uuid_type(&f.ty) && f.ident.as_ref().unwrap() == "id"))
		.map(|f| f.ident.as_ref().unwrap().to_string())
		.collect();

	let value_exprs: Vec<proc_macro2::TokenStream> = fields
		.iter()
		.map(|f| {
			let ident = &f.ident;
			let ty = &f.ty;

			if is_uuid_type(ty) && ident.as_ref().unwrap() == "id" {
				quote! { some_sqlite::QueryValue::String(Uuid::new_v4().to_string()) }
			} else if is_option_type(ty) {
				quote! { self.#ident.clone().map(|v| v.into()).unwrap_or(some_sqlite::QueryValue::Null) }
			} else if is_uuid_type(ty) {
				quote! { some_sqlite::QueryValue::String(self.#ident.to_string()) }
			} else {
				quote! { self.#ident.clone().into() }
			}
		})
		.collect();

	let expanded = quote! {
		impl some_sqlite::NewEntity for #name {
			type Entity = #entity_type;

			fn table_name() -> &'static str {
				#entity_type::table_name()
			}

			fn columns_and_values(&self) -> (Vec<&str>, Vec<some_sqlite::QueryValue>) {
				let columns: Vec<&str> = vec![#(#column_names),*];
				let values: Vec<some_sqlite::QueryValue> = vec![#(#value_exprs),*];
				(columns, values)
			}
		}
	};

	TokenStream::from(expanded)
}

// ---------------- Schema Macro ----------------

// #[proc_macro_derive(Schema, attributes(schema, table_name, indexes, setup_sql))]
// pub fn derive_schema(input: TokenStream) -> TokenStream {
// 	let input = parse_macro_input!(input as DeriveInput);
// 	let name = &input.ident;
// 	let table_name = get_table_name(&input.attrs, &name.to_string());
// 	let create_sql = generate_create_table_sql(&input, &table_name);
// 	let create_sql_literal = Box::leak(create_sql.into_boxed_str());
//
// 	let expanded = quote! {
// 		impl some_sqlite::Schema for #name {
// 			fn create_table_sql() -> &'static str {
// 				#create_sql_literal
// 			}
//
// 			fn indexes() -> Vec<&'static str> {
// 				vec![]
// 			}
//
// 			fn setup_sql() -> Vec<&'static str> {
// 				vec![]
// 			}
// 		}
// 	};
// 	TokenStream::from(expanded)
// }

// ---------------------- Helper functions ----------------------

fn get_table_name(attrs: &[Attribute], default_name: &str) -> String {
	for attr in attrs {
		if attr.path.is_ident("table_name") {
			if let Ok(value) = attr.parse_args::<syn::LitStr>() {
				return value.value();
			}
		}
	}
	to_snake_case(default_name)
}

fn get_primary_key_info(input: &DeriveInput) -> (syn::Ident, Type) {
	if let Data::Struct(data_struct) = &input.data {
		if let Fields::Named(fields) = &data_struct.fields {
			for field in &fields.named {
				if field.attrs.iter().any(|attr| attr.path.is_ident("primary_key")) {
					return (field.ident.as_ref().unwrap().clone(), field.ty.clone());
				}
			}
			// fallback to "id"
			for field in &fields.named {
				if let Some(ident) = &field.ident {
					if ident == "id" {
						return (ident.clone(), field.ty.clone());
					}
				}
			}
		}
	}
	panic!("Could not find primary key field. Use #[primary_key] or name it 'id'");
}

fn is_uuid_type(ty: &Type) -> bool {
	if let Type::Path(tp) = ty {
		return tp.path.segments.last().unwrap().ident == "Uuid";
	}
	false
}

fn is_option_type(ty: &Type) -> bool {
	if let Type::Path(tp) = ty {
		return tp.path.segments.last().unwrap().ident == "Option";
	}
	false
}

// Parse entity attribute for NewEntity
fn get_entity_attr(attrs: &[Attribute]) -> syn::Ident {
	for attr in attrs {
		if attr.path.is_ident("new_entity") {
			if let Ok(syn::Meta::List(meta)) = attr.parse_meta() {
				for nested in meta.nested.iter() {
					if let syn::NestedMeta::Meta(syn::Meta::NameValue(nv)) = nested {
						if nv.path.is_ident("entity") {
							if let syn::Lit::Str(litstr) = &nv.lit {
								return syn::Ident::new(&litstr.value(), litstr.span());
							}
						}
					}
				}
			}
		}
	}
	panic!("Missing `entity = \"...\"` in #[new_entity(...)]");
}

fn to_snake_case(input: &str) -> String {
	let mut result = String::new();
	let mut chars = input.chars().peekable();
	while let Some(ch) = chars.next() {
		if ch.is_uppercase() && !result.is_empty() {
			result.push('_');
		}
		result.push(ch.to_lowercase().next().unwrap());
	}
	result
}
