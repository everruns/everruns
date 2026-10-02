//! The everruns engine inside a celld Durable Object.
//!
//! Each cell (one Durable Object, `/cells/<name>/...`) is one agent session:
//! its event log lives in the cell's SQLite, and its turn runs inside the cell,
//! one engine step per commit. Losing the node mid-turn costs at most the step
//! that was running; the cell's alarm resumes the turn on whichever node owns
//! the cell next. See `README.md`.
//!
//! The step machine ([`cell`]) and the driver ([`openai`]) are portable and
//! tested natively; only [`durable`] needs the JavaScript isolate.

pub mod agent;
pub mod cell;
pub mod openai;

#[cfg(target_arch = "wasm32")]
mod durable;

#[cfg(test)]
mod tests;
