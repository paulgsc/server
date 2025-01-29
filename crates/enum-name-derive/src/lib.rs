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

	let expanded = quote! {
	impl #name {
						#[must_use]
						pub fn filename(&self) -> &str {
												match self {
																			#(#match_arms,)*
																	}
														}
							}
	};

	TokenStream::from(expanded)
}
