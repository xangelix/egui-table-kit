//! Search matching structures with support for Regex and case-insensitive lookups.

use fluent_zero::t;

bitflags::bitflags! {
    /// Configuration flags for search query evaluations.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub struct SearchOptions: u8 {
        /// Evaluates text comparisons case-insensitively.
        const CASE_INSENSITIVE = 0b0000_0001;
        /// Compiles the search query as a regular expression.
        const REGEX            = 0b0000_0010;
    }
}

/// Internal state for the matching logic.
#[derive(Debug, Clone, Default)]
enum Matcher {
    /// Matches everything (empty query or inactive).
    #[default]
    Always,
    /// Fast exact substring match (Case-Sensitive Text).
    Literal(String),
    /// Compiled Regex match.
    Compiled(regex::Regex),
    /// The user provided an invalid regex.
    Invalid(String),
}

/// A comprehensive search engine supporting regex queries and case insensitivity.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Search {
    raw_query: String,
    active: bool,
    options: SearchOptions,
    #[serde(skip)]
    matcher: Matcher,
}

impl Search {
    /// Creates a new inactive search container.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluates if the provided text satisfies the current search filter.
    #[must_use]
    pub fn is_match(&self, text: &str) -> bool {
        if !self.active {
            return true;
        }

        match &self.matcher {
            Matcher::Always => true,
            Matcher::Literal(s) => text.contains(s),
            Matcher::Compiled(re) => re.is_match(text),
            Matcher::Invalid(_) => false,
        }
    }

    /// Updates the raw query string and regenerates the matcher state.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.raw_query = text.into();
        self.rebuild_matcher();
    }

    /// Replaces the active search options.
    pub fn set_options(&mut self, options: SearchOptions) {
        if self.options != options {
            self.options = options;
            self.rebuild_matcher();
        }
    }

    /// Toggles a specific search behavior option.
    pub fn toggle_option(&mut self, option: SearchOptions) {
        self.options.toggle(option);
        self.rebuild_matcher();
    }

    /// Activates the search matching engine.
    pub fn open(&mut self) {
        if !self.active {
            self.active = true;
            self.rebuild_matcher();
        }
    }

    /// Clears the query and deactivates matching.
    pub fn clear(&mut self) {
        self.raw_query.clear();
        self.active = false;
        self.matcher = Matcher::Always;
    }

    /// Permits safe mutability of the search query, rebuilding state on changes.
    pub fn edit_text(&mut self, f: impl FnOnce(&mut String) -> bool) -> bool {
        let changed = f(&mut self.raw_query);
        if changed {
            self.rebuild_matcher();
        }
        changed
    }

    /// Exposes the current raw query string.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.raw_query
    }

    /// Exposes the current search configuration options.
    #[must_use]
    pub const fn options(&self) -> SearchOptions {
        self.options
    }

    /// Returns whether the search is active.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// Exposes localized error messages if a regex query fails compilation.
    #[must_use]
    pub fn error_message(&self) -> Option<&str> {
        if let Matcher::Invalid(msg) = &self.matcher {
            Some(msg)
        } else {
            None
        }
    }

    fn rebuild_matcher(&mut self) {
        if self.raw_query.is_empty() {
            self.matcher = Matcher::Always;
            return;
        }

        let case_insensitive = self.options.contains(SearchOptions::CASE_INSENSITIVE);
        let is_regex_mode = self.options.contains(SearchOptions::REGEX);

        if !is_regex_mode && !case_insensitive {
            self.matcher = Matcher::Literal(self.raw_query.clone());
            return;
        }

        let pattern = if is_regex_mode {
            self.raw_query.clone()
        } else {
            regex::escape(&self.raw_query)
        };

        match regex::RegexBuilder::new(&pattern)
            .case_insensitive(case_insensitive)
            .build()
        {
            Ok(re) => {
                self.matcher = Matcher::Compiled(re);
            }
            Err(e) => {
                self.matcher = Matcher::Invalid(e.to_string());
            }
        }
    }
}

/// Helper component to render search options and query input widgets.
pub struct SearchBar<'a> {
    label: &'a str,
}

impl<'a> SearchBar<'a> {
    /// Creates a new search bar UI constructor.
    #[must_use]
    pub const fn new(label: &'a str) -> Self {
        Self { label }
    }

    /// Renders search interaction elements, returning `true` if state parameters changed.
    pub fn ui(self, ui: &mut egui::Ui, search: &mut Search) -> bool {
        let mut changed = false;

        ui.horizontal(|ui| {
            if search.is_active() {
                let text_changed = search.edit_text(|s| {
                    ui.add(
                        egui::TextEdit::singleline(s)
                            .clip_text(true)
                            .hint_text("Search..."),
                    )
                    .changed()
                });

                if text_changed {
                    changed = true;
                }

                let case_selected = search.options().contains(SearchOptions::CASE_INSENSITIVE);
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("Aa").monospace())
                            .selected(case_selected),
                    )
                    .on_hover_text(t!("case-insensitive"))
                    .clicked()
                {
                    search.toggle_option(SearchOptions::CASE_INSENSITIVE);
                    changed = true;
                }

                let regex_selected = search.options().contains(SearchOptions::REGEX);
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new(".*").monospace())
                            .selected(regex_selected),
                    )
                    .on_hover_text(t!("regular-expression"))
                    .clicked()
                {
                    search.toggle_option(SearchOptions::REGEX);
                    changed = true;
                }

                if ui.button("❌").on_hover_text(t!("remove-filter")).clicked() {
                    search.clear();
                    changed = true;
                }
            } else if ui.button(self.label).clicked() {
                search.open();
                changed = true;
            }

            if search.is_active()
                && let Some(msg) = search.error_message()
            {
                ui.label(
                    egui::RichText::new(format!("⚠ {msg}"))
                        .monospace()
                        .color(egui::Color32::RED),
                );
            }
        });

        changed
    }
}
