//! Shared text/search primitives for frona.

mod line_ending;
mod normalized_string;
mod search;

pub use line_ending::LineEnding;
pub use normalized_string::NormalizedString;
pub use search::walk_with_ignore;
