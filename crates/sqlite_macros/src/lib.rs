use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, Attribute, DeriveInput, Lit, Meta, NestedMeta};

#[derive(Default)]
struct SqliteTypeOpts {
	validate: bool,
	max_length: Option<usize>,
	custom_error: Option<String>,
	convert_from: Option<String>,
}

fn parse_attributes(attrs: &[Attribute]) -> SqliteTypeOpts {
	let mut opts = SqliteTypeOpts::default();

	for attr in attrs {
		if !attr.path.is_ident("sqlite_type") {
			continue;
		}

		if let Ok(Meta::List(meta_list)) = attr.parse_meta() {
			for nested in meta_list.nested {
				if let NestedMeta::Meta(Meta::NameValue(nv)) = nested {
					if nv.path.is_ident("validate") {
						if let Lit::Bool(lit) = nv.lit {
							opts.validate = lit.value;
						}
					} else if nv.path.is_ident("max_length") {
						if let Lit::Int(lit) = nv.lit {
							opts.max_length = Some(lit.base10_parse().unwrap_or(0));
						}
					} else if nv.path.is_ident("error") {
						if let Lit::Str(lit) = nv.lit {
							opts.custom_error = Some(lit.value());
						}
					} else if nv.path.is_ident("convert_from") {
						if let Lit::Str(lit) = nv.lit {
							opts.convert_from = Some(lit.value());
						}
					}
				}
			}
		}
	}
	opts
}

#[proc_macro_derive(SqliteType, attributes(sqlite_type))]
pub fn derive_sqlite_type(input: TokenStream) -> TokenStream {
	let input = parse_macro_input!(input as DeriveInput);
	let name = &input.ident;
	let opts = parse_attributes(&input.attrs);

	// Define error type
	let error_type = format_ident!("{}Error", name);
	let error_msg = opts.custom_error.clone().unwrap_or_else(|| format!("Invalid {} value", name));

	// Generate validation function
	let validation = if opts.validate {
		let max_length_check = if let Some(max_len) = opts.max_length {
			quote! {
					if value.len() > #max_len {
							return Err(#error_type::Invalid(format!("{} exceeds maximum length of {}", stringify!(#name), #max_len)));
					}
			}
		} else {
			quote! {}
		};

		quote! {
				fn validate(value: &str) -> Result<(), #error_type> {
						if value.is_empty() {
								return Err(#error_type::Invalid(format!("{} cannot be empty", stringify!(#name))));
						}
						#max_length_check
						Ok(())
				}
		}
	} else {
		quote! {}
	};

	// Handle conversion from another type (if applicable)
	let conversion = if let Some(from_type) = opts.convert_from {
		let from_type = format_ident!("{}", from_type);
		quote! {
				impl TryFrom<#from_type> for #name {
						type Error = #error_type;

						fn try_from(value: #from_type) -> Result<Self, Self::Error> {
								let str_value = value.to_string();
								if Self::validate(&str_value).is_ok() {
										Ok(Self(str_value.into()))
								} else {
										Err(#error_type::ConversionFailed(#error_msg.to_string()))
								}
						}
				}
		}
	} else {
		quote! {}
	};

	// Generate the expanded code for the macro
	let expanded = quote! {
			#[derive(Debug)]
			pub enum #error_type {
					Invalid(String),
					ConversionFailed(String),
					Database(sqlx::Error),
					Parse(String),
			}

			impl std::fmt::Display for #error_type {
					fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
							match self {
									Self::Invalid(msg) => write!(f, "Invalid {}: {}", stringify!(#name), msg),
									Self::ConversionFailed(msg) => write!(f, "Conversion failed: {}", msg),
									Self::Database(e) => write!(f, "Database error: {}", e),
									Self::Parse(e) => write!(f, "Parse error: {}", e),
							}
					}
			}

			impl std::error::Error for #error_type {}

			impl From<sqlx::Error> for #error_type {
					fn from(err: sqlx::Error) -> Self {
							Self::Database(err)
					}
			}

			impl #name {
					#validation
			}

			impl sqlx::Type<sqlx::Sqlite> for #name {
					fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
							<String as sqlx::Type<sqlx::Sqlite>>::type_info()
					}

					fn compatible(ty: &sqlx::sqlite::SqliteTypeInfo) -> bool {
							<String as sqlx::Type<sqlx::Sqlite>>::compatible(ty)
					}
			}

			impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for #name {
					fn encode_by_ref(&self, args: &mut <sqlx::Sqlite as sqlx::database::HasArguments<'q>>::ArgumentBuffer) -> sqlx::encode::IsNull {
							use std::string::ToString;
							<String as sqlx::Encode<sqlx::Sqlite>>::encode_by_ref(&self.to_string(), args)
					}
			}

			impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for #name {
					fn decode(value: sqlx::sqlite::SqliteValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
							let s = <String as sqlx::Decode<sqlx::Sqlite>>::decode(value)?;
							s.parse::<Self>()
									.map_err(|e| Box::new(#error_type::Parse(e.to_string())) as Box<dyn std::error::Error + Send + Sync>)
					}
			}

			#conversion
	};

	// Return the generated code as TokenStream
	TokenStream::from(expanded)
}

#[proc_macro_derive(SqliteValidatedType)]
pub fn derive_sqlite_validated_type(input: TokenStream) -> TokenStream {
	let input = parse_macro_input!(input as DeriveInput);
	let name = &input.ident;

	let expanded = quote! {

			impl Encode<'_, Sqlite> for #name {
					fn encode_by_ref(&self, buf: &mut <Sqlite as HasArguments>::ArgumentBuffer) -> IsNull {
							if let Err(e) = self.validate() {
									eprintln!("Warning: encoding invalid {}: {}", stringify!(#name), e);
							}
							self.to_string().encode(buf)
					}
			}

			impl<'r> Decode<'r, Sqlite> for #name {
					fn decode(value: SqliteValueRef<'r>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
							let decoded = <String as Decode<Sqlite>>::decode(value)?;
							let instance: Self = decoded.parse()?;

							instance.validate()
									.map_err(|e| format!("Validation failed for {}: {}", stringify!(#name), e))?;

							Ok(instance)
					}
			}
	};

	TokenStream::from(expanded)
}

//
// #[proc_macro_derive(ConvertI32toI64)]
// pub fn convert_i32_to_i64(input: TokenStream) -> TokenStream {
// 	let input = parse_macro_input!(input as DeriveInput);
//
// 	// Extract the struct's name
// 	let struct_name = input.ident;
//
// 	// Match on the data type (should be a struct)
// 	if let Data::Struct(data) = input.data {
// 		let fields = data.fields.iter().map(|field| {
// 			let field_name = &field.ident;
// 			let field_type = &field.ty;
//
// 			// Check if the field is an i32, and replace with i64 if true
// 			let new_type = if let Type::Path(type_path) = field_type {
// 				if type_path.path.is_ident("i32") {
// 					quote! { i64 }
// 				} else {
// 					quote! { #field_type }
// 				}
// 			} else {
// 				quote! { #field_type }
// 			};
//
// 			// Generate the new field declaration
// 			quote! {
// 					pub #field_name: #new_type,
// 			}
// 		});
//
// 		// Generate the output struct with the modified field types
// 		let expanded = quote! {
// 				pub struct #struct_name {
// 						#(#fields)*
// 				}
// 		};
//
// 		// Convert the generated tokens back to TokenStream
// 		TokenStream::from(expanded)
// 	} else {
// 		// If the input isn't a struct, return an error
// 		syn::Error::new(input.span(), "Only structs are supported").to_compile_error().into()
// 	}
// }

#[cfg(test)]
mod tests {
	use super::*;
	use sqlx::sqlite::SqliteRow;
	use sqlx::{Row, Sqlite};

	#[test]
	fn test_decode_invalid_data() {
		#[derive(Debug, PartialEq)]
		struct MockType(String);

		impl MockType {
			fn validate(value: &str) -> Result<(), String> {
				if value.len() > 5 {
					Err("Value exceeds maximum length".to_string())
				} else {
					Ok(())
				}
			}
		}

		impl<'r> Decode<'r, Sqlite> for MockType {
			fn decode(value: SqliteValueRef<'r>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
				let s = <String as Decode<Sqlite>>::decode(value)?;
				MockType::validate(&s).map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
				Ok(MockType(s))
			}
		}

		let invalid_value = "toolong";
		let decode_result = MockType::decode(SqliteValueRef::from(invalid_value));
		assert!(decode_result.is_err());
	}
}
