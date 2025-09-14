use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Attribute, Data, DeriveInput, Fields, Type};

/// Derive macro for Entity trait
#[proc_macro_derive(Entity, attributes(entity, primary_key, table_name))]
pub fn derive_entity(input: TokenStream) -> TokenStream {
	let input = parse_macro_input!(input as DeriveInput);
	let name = &input.ident;

	let table_name = get_table_name(&input.attrs, &name.to_string());
	let (pk_field, pk_type) = get_primary_key_info(&input);

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
		}
	};

	TokenStream::from(expanded)
}

/// Derive macro for NewEntity trait
#[proc_macro_derive(NewEntity, attributes(new_entity, table_name))]
pub fn derive_new_entity(input: TokenStream) -> TokenStream {
	let input = parse_macro_input!(input as DeriveInput);
	let name = &input.ident;

	let entity_type = get_entity_type(&input.attrs).unwrap_or_else(|| format!("{}Entity", name));
	let entity_type = syn::parse_str::<Type>(&entity_type).unwrap();

	let expanded = quote! {
		impl some_sqlite::NewEntity for #name {
			type Entity = #entity_type;
		}
	};

	TokenStream::from(expanded)
}

/// Derive macro for Schema trait
#[proc_macro_derive(Schema, attributes(schema, table_name, indexes, setup_sql))]
pub fn derive_schema(input: TokenStream) -> TokenStream {
	let input = parse_macro_input!(input as DeriveInput);
	let name = &input.ident;

	let table_name = get_table_name(&input.attrs, &name.to_string());
	let create_sql = generate_create_table_sql(&input, &table_name);
	let indexes = get_indexes(&input.attrs);
	let setup_sql = get_setup_sql(&input.attrs);

	let expanded = quote! {
		impl some_sqlite::Schema for #name {
			fn create_table_sql() -> &'static str {
				#create_sql
			}

			fn indexes() -> Vec<&'static str> {
				vec![#(#indexes),*]
			}

			fn setup_sql() -> Vec<&'static str> {
				vec![#(#setup_sql),*]
			}
		}
	};

	TokenStream::from(expanded)
}

/// Macro to generate a complete CRUD setup for an entity
#[proc_macro]
pub fn sqlite_entity(input: TokenStream) -> TokenStream {
	let input = parse_macro_input!(input as syn::ItemStruct);
	let name = &input.ident;
	let new_entity_name = syn::Ident::new(&format!("New{}", name), name.span());

	let fields = match &input.fields {
		Fields::Named(fields) => &fields.named,
		_ => panic!("sqlite_entity only supports structs with named fields"),
	};

	// Generate the NewEntity struct (without the primary key)
	let new_fields: Vec<_> = fields.iter().filter(|field| !is_primary_key_field(field)).collect();

	let table_name = get_table_name(&input.attrs, &name.to_string());
	let create_sql = generate_create_table_sql_from_struct(&input, &table_name);

	let expanded = quote! {
		#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
		pub struct #new_entity_name {
			#(pub #new_fields),*
		}

		impl some_sqlite::NewEntity for #new_entity_name {
			type Entity = #name;
		}

		impl some_sqlite::Schema for #name {
			fn create_table_sql() -> &'static str {
				#create_sql
			}
		}
	};

	TokenStream::from(expanded)
}

// Helper functions
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
			// Look for #[primary_key] attribute first
			for field in &fields.named {
				if field.attrs.iter().any(|attr| attr.path.is_ident("primary_key")) {
					return (field.ident.as_ref().unwrap().clone(), field.ty.clone());
				}
			}

			// Fallback to field named "id"
			for field in &fields.named {
				if let Some(ident) = &field.ident {
					if ident == "id" {
						return (ident.clone(), field.ty.clone());
					}
				}
			}
		}
	}

	panic!("Could not find primary key field. Either use #[primary_key] or name the field 'id'");
}

fn get_entity_type(attrs: &[Attribute]) -> Option<String> {
	for attr in attrs {
		if attr.path.is_ident("new_entity") {
			if let Ok(value) = attr.parse_args::<syn::LitStr>() {
				return Some(value.value());
			}
		}
	}
	None
}

fn get_indexes(attrs: &[Attribute]) -> Vec<String> {
	for attr in attrs {
		if attr.path.is_ident("indexes") {
			if let Ok(array) = attr.parse_args::<syn::ExprArray>() {
				return array
					.elems
					.iter()
					.filter_map(|expr| {
						if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) = expr {
							Some(lit_str.value())
						} else {
							None
						}
					})
					.collect();
			}
		}
	}
	vec![]
}

fn get_setup_sql(attrs: &[Attribute]) -> Vec<String> {
	for attr in attrs {
		if attr.path.is_ident("setup_sql") {
			if let Ok(array) = attr.parse_args::<syn::ExprArray>() {
				return array
					.elems
					.iter()
					.filter_map(|expr| {
						if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(lit_str), .. }) = expr {
							Some(lit_str.value())
						} else {
							None
						}
					})
					.collect();
			}
		}
	}
	vec![]
}

fn generate_create_table_sql(input: &DeriveInput, table_name: &str) -> String {
	if let Data::Struct(data_struct) = &input.data {
		if let Fields::Named(fields) = &data_struct.fields {
			let mut columns = Vec::new();
			let mut _primary_key = None;

			for field in &fields.named {
				let field_name = field.ident.as_ref().unwrap().to_string();
				let sql_type = rust_type_to_sql_type(&field.ty);

				let mut column_def = format!("{} {}", field_name, sql_type);

				// Check for primary key
				if is_primary_key_field(field) || field_name == "id" {
					column_def.push_str(" PRIMARY KEY");
					_primary_key = Some(field_name.clone());
				}

				// Check for NOT NULL (if not Option<T>)
				if !is_optional_type(&field.ty) {
					column_def.push_str(" NOT NULL");
				}

				columns.push(column_def);
			}

			let create_sql = format!("CREATE TABLE IF NOT EXISTS {} ({})", table_name, columns.join(", "));

			return create_sql;
		}
	}

	panic!("Schema derive only supports structs with named fields");
}

fn generate_create_table_sql_from_struct(input: &syn::ItemStruct, table_name: &str) -> String {
	if let Fields::Named(fields) = &input.fields {
		let mut columns = Vec::new();

		for field in &fields.named {
			let field_name = field.ident.as_ref().unwrap().to_string();
			let sql_type = rust_type_to_sql_type(&field.ty);

			let mut column_def = format!("{} {}", field_name, sql_type);

			// Check for primary key
			if is_primary_key_field(field) || field_name == "id" {
				column_def.push_str(" PRIMARY KEY");
			}

			// Check for NOT NULL (if not Option<T>)
			if !is_optional_type(&field.ty) {
				column_def.push_str(" NOT NULL");
			}

			columns.push(column_def);
		}

		return format!("CREATE TABLE IF NOT EXISTS {} ({})", table_name, columns.join(", "));
	}

	panic!("sqlite_entity only supports structs with named fields");
}

fn is_primary_key_field(field: &syn::Field) -> bool {
	field.attrs.iter().any(|attr| attr.path.is_ident("primary_key"))
}

fn is_optional_type(ty: &Type) -> bool {
	if let Type::Path(type_path) = ty {
		if let Some(segment) = type_path.path.segments.last() {
			return segment.ident == "Option";
		}
	}
	false
}

fn rust_type_to_sql_type(ty: &Type) -> &'static str {
	// This is a simplified mapping - you might want to expand this
	if let Type::Path(type_path) = ty {
		if let Some(segment) = type_path.path.segments.last() {
			match segment.ident.to_string().as_str() {
				"String" => "TEXT",
				"i32" | "i64" => "INTEGER",
				"f32" | "f64" => "REAL",
				"bool" => "BOOLEAN",
				"Option" => {
					// For Option<T>, we need to look at T
					if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
						if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
							return rust_type_to_sql_type(inner_ty);
						}
					}
					"TEXT"
				}
				"DateTime" => "INTEGER", // Store as timestamp
				"Uuid" => "TEXT",
				"Vec" => "TEXT",     // JSON
				"HashMap" => "TEXT", // JSON
				_ => "TEXT",         // Default fallback
			}
		} else {
			"TEXT"
		}
	} else {
		"TEXT"
	}
}

fn to_snake_case(input: &str) -> String {
	let mut result = String::new();
	let mut chars = input.chars().peekable();

	while let Some(ch) = chars.next() {
		if ch.is_uppercase() {
			if !result.is_empty() && chars.peek().is_some() {
				result.push('_');
			}
			result.push(ch.to_lowercase().next().unwrap());
		} else {
			result.push(ch);
		}
	}

	result
}
