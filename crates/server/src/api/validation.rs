// Input validation lives in `crate::domains::validation` so domains need not
// import the HTTP layer; re-exported here for API handlers and SaaS callers.
pub use crate::domains::validation::*;
