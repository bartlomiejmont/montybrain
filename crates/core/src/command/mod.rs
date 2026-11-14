//! The command language of Skrzynka, the quick-capture window, ⌘K and the MCP
//! `command` tool: one parser and resolver for all of them.
//!
//! * [`parse`] — text → syntax (no vault);
//! * [`preview`] — syntax + snapshot → [`Preview`] (the reading, the plan with
//!   the exact Markdown lines, warnings, choices);
//! * [`run`] — re-check the plan against the vault and write it;
//! * [`undo`] — take a write back while it is unchanged;
//! * [`complete`] — autocomplete for the word under the caret;
//! * [`suggest`] — local destination suggestions and the duplicate hint.
//!
//! The grammar and its rules are in `parse.rs` and `docs/format.md` §13.

pub mod complete;
pub mod dates;
pub mod exec;
pub mod marks;
pub mod parse;
pub mod plan;
pub mod resolve;
pub mod suggest;

pub use complete::{Completions, Item, complete};
pub use exec::{
    Applied, Outcome, Refusal, StepResult, UndoOutcome, UndoStatus, UndoStep, run, undo,
};
pub use marks::mark_ranges;
pub use parse::{Span, parse};
pub use plan::{
    AgentAsk, Context, Fields, LineTarget, Op, Plan, Preview, SpanChoice, State, Step, TaskBasis,
    plan_token, planned_id, preview,
};
pub use resolve::{Choice, Hit, Place, resolve_search, search};
pub use suggest::{
    Candidate, InboxSuggestion, Similar, candidates, inbox_suggestions, similar_tasks,
};
