use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Lit, Meta};

#[proc_macro_derive(EnumFilenameAndFromString, attributes(filename))]
pub fn derive_enum_filename_and_from_string(input: TokenStream) -> TokenStream {
	let input = parse_macro_input!(input as DeriveInput);
	let name = &input.ident;

	let data = if let syn::Data::Enum(data) = input.data {
		data
	} else {
		panic!("EnumFilenameAndFromString can only be derived for enums");
	};

	let filename_match_arms = data.variants.iter().map(|variant| {
		let variant_name = &variant.ident;
		let filename = get_filename_from_attributes(variant);
		quote! {
		Self::#variant_name => #filename
				}
	});

	let from_str_match_arms = data.variants.iter().map(|variant| {
		let variant_name = &variant.ident;
		let filename = get_filename_from_attributes(variant);
		quote! {
		#filename => Ok(Self::#variant_name),
			}
	});

	let expanded = quote! {
	impl #name {
						#[must_use]
						pub fn filename(&self) -> &str {
												match self {
																			#(#filename_match_arms,)*
																	}
														}
							}

			impl std::str::FromStr for #name {
								type Err = String;

											fn from_str(s: &str) -> Result<Self, Self::Err> {
																	match s {
																								#(#from_str_match_arms)*
																								_ => Err(format!("'{}' is not a valid {} filename", s, stringify!(#name))),
																											}
																			}
												}
			};

	TokenStream::from(expanded)
}

fn get_filename_from_attributes(variant: &syn::Variant) -> String {
	variant
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
		.expect(&format!("Missing filename attribute for variant {}", variant.ident))
}
