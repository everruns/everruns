//! System One wire values shared by credential-free protocol drivers.
pub mod answer;
pub mod error;
pub mod question;
pub use answer::{Answer, ChoiceAnswer, Judgment, NoulAnswer, ScoreAnswer, Usage};
pub use error::{Error, Result};
pub use question::{DEFAULT_MODEL, Evaluation, NoulCriteria, Question};
