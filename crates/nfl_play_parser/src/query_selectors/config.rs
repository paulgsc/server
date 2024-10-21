pub mod config {
	#[derive(Debug)]
	pub enum PlaySelectors {
		PlayList,
		PlayListItem,
		Description,
		Headline,
		TeamLogo,
	}

	impl PlaySelectors {
		pub fn selector(&self) -> &'static str {
			match self {
				PlaySelectors::PlayList => ".AccordionPanel",
				PlaySelectors::PlayListItem => ".PlayListItem",
				PlaySelectors::Description => ".PlayListItem__Description",
				PlaySelectors::Headline => ".PlayListItem__Headline",
				PlaySelectors::TeamLogo => "img.AccordionHeader__Left__TeamLogo",
			}
		}
	}
}
