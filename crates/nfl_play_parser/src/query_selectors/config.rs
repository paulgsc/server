pub mod config {
    #[derive(Debug)]
    pub enum PlaySelectors {
        PlayList,
        PlayListItem,
        Description,
        Headline,
    }

    impl PlaySelectors {
        pub fn selector(&self) -> &'static str {
            match self {
                PlaySelectors::PlayList => ".PlayList",
                PlaySelectors::PlayListItem => ".PlayListItem",
                PlaySelectors::Description => ".PlayListItem__Description",
                PlaySelectors::Headline => ".PlayListItem__Headline",
            }
        }
    }
}

