// Shared configuration helpers
//
// Decision: Helper functions over traits — configs are too heterogeneous for a
// single trait to add value. Composable functions give the same deduplication
// with zero abstraction overhead.
//
// Decision: No derive macro — the complexity of a proc macro isn't justified
// for ~49 structs with varying patterns. Plain functions are transparent and
// easy to audit.

mod env;
mod error;

pub use env::*;
pub use error::ConfigError;
