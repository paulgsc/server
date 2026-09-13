macro_rules! owned_format {
	($($arg:tt)*) => {{
		use std::fmt::Write as _;
		let mut output = String::new();
		write!(&mut output, $($arg)*).expect("writing to a String cannot fail");
		output
	}};
}

#[cfg(feature = "ws-events")]
pub mod events;

#[cfg(feature = "ws-events")]
pub use events::{unified_event, UnifiedEvent};

#[cfg(feature = "tabsched")]
pub mod tabsched;
