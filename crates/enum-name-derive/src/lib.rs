use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Lit, Meta};

#[proc_macro_derive(EnumFilename, attributes(filename))]
pub fn derive_enum_filename(input: TokenStream) -> TokenStream {
	let input = parse_macro_input!(input as DeriveInput);

	let name = &input.ident;

	let data = if let syn::Data::Enum(data) = input.data {
		data
	} else {
		panic!("EnumFilename can only be derived for enums");
	};

	// Generate match arms for the `filename` method
	let match_arms = data.variants.iter().map(|variant| {
		let variant_name = &variant.ident;
		let filename = variant
			.attrs
			.iter()
			.find(|attr| attr.path.is_ident("filename"))
			.and_then(|attr| {
				if let Ok(Meta::NameValue(meta)) = attr.parse_meta() {
					if let Lit::Str(lit) = meta.lit {
						Some(lit.value())
					} else {
						None
					}
				} else {
					None
				}
			})
			.expect(&format!("Missing filename attribute for variant {}", variant_name));

		quote! {
		Self::#variant_name => #filename
				}
	});

	// Generate match arms for the `FromStr` implementation
	let from_str_match_arms = data.variants.iter().map(|variant| {
		let variant_name = &variant.ident;
		let filename = variant
			.attrs
			.iter()
			.find(|attr| attr.path.is_ident("filename"))
			.and_then(|attr| {
				if let Ok(Meta::NameValue(meta)) = attr.parse_meta() {
					if let Lit::Str(lit) = meta.lit {
						Some(lit.value())
					} else {
						None
					}
				} else {
					None
				}
			})
			.expect(&format!("Missing filename attribute for variant {}", variant_name));

		quote! {
			#filename => Ok(Self::#variant_name),
		}
	});

	// Define the error type for `FromStr`
	let error_type = quote! {
		#[derive(Debug)]
		pub struct ParseEnumError;

		impl std::fmt::Display for ParseEnumError {
			fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				write!(f, "failed to parse enum from string")
			}
		}

		impl std::error::Error for ParseEnumError {}
	};

	// Generate the expanded code
	let expanded = quote! {
		impl #name {
			#[must_use]
			pub fn filename(&self) -> &str {
				match self {
					#(#match_arms,)*
				}
			}
		}

		impl std::str::FromStr for #name {
			type Err = ParseEnumError;

			fn from_str(s: &str) -> Result<Self, Self::Err> {
				match s {
					#(#from_str_match_arms)*
					_ => Err(ParseEnumError),
				}
			}
		}

		#error_type
	};

	TokenStream::from(expanded)
}
