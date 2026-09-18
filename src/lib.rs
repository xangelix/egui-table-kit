pub mod delegate;
pub mod error;
pub mod filter;
pub mod header;
pub mod highlights;
pub mod interaction;
pub mod layout;
pub mod operations;
pub mod state;
pub mod table;

#[cfg(test)]
mod interaction_tests;

include!(concat!(env!("OUT_DIR"), "/static_cache.rs"));
