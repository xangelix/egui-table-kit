pub mod highlight;
pub mod search;

/// A composable column filter merging search and highlight bounds.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Filter {
    pub search: search::Search,
    pub highlight: Option<Option<u8>>,
}

impl Filter {
    #[must_use]
    pub fn matches(&self, text: &str, row_highlight: Option<u8>) -> bool {
        if let Some(req_highlight) = self.highlight
            && row_highlight != req_highlight
        {
            return false;
        }
        self.search.is_match(text)
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        !self.search.is_active() && self.highlight.is_none()
    }
}
