#![allow(clippy::cast_abs_to_unsigned)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::comparison_chain)]
#![allow(clippy::disallowed_macros)]
#![allow(clippy::elidable_lifetime_names)]
#![allow(clippy::fallible_impl_from)]
#![allow(clippy::if_not_else)]
#![allow(clippy::manual_string_new)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::module_inception)]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::new_without_default)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::struct_field_names)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::unnecessary_lazy_evaluations)]
#![allow(clippy::use_self)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::derive_partial_eq_without_eq)]
pub mod error;
pub mod query_selectors;
pub mod schema;

use file_reader::{FileReader, FileReaderError};
use scraper::Html;

pub fn read_html_file(file_path: &str) -> Result<Html, FileReaderError> {
	let reader = FileReader::new(file_path, "html")?;
	let html_content = reader.read_content()?;
	Ok(Html::parse_document(&html_content))
}
