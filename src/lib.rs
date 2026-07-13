pub mod delegate;
pub mod error;
pub mod filter;
pub mod header;
pub mod highlights;
pub mod layout;
pub mod operations;
pub mod state;
pub mod table;

include!(concat!(env!("OUT_DIR"), "/static_cache.rs"));
