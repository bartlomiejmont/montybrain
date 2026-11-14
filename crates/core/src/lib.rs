//! # morwisko-core
//!
//! Plain-Markdown vault for Morwisko: projects → topics → tasks, with a journal
//! per topic. This crate owns the on-disk format (see `docs/format.md`) and is the
//! only thing that should write vault files.
//!
//! * [`Vault`] — open/init a vault, [`Vault::load`] a [`Snapshot`], and mutate it
//!   (every mutation is lock → re-read → edit → atomic write).
//! * [`Snapshot`] — in-memory model plus queries ([`Snapshot::tasks`],
//!   [`Snapshot::search`], [`Snapshot::today`], [`Snapshot::recent_journal`]).
//! * Parsing is lossless: files are kept as lines, so untouched files round-trip
//!   byte for byte and edits only change the lines they concern.
//! * Feature `watch`: [`Vault::watch`] (debounced `notify` watcher).

pub mod check;
mod doc;
mod error;
mod files;
mod frontmatter;
mod id;
mod journal;
mod model;
mod query;
mod slug;
mod task;
mod validate;
mod vault;
mod verify;
#[cfg(feature = "watch")]
mod watch;

pub use error::{Error, Result};
pub use id::{ID_LEN, is_valid_id};
pub use model::*;
pub use query::{SearchHit, Snapshot, TaskFilter, TodayItem, TodayReason};
pub use slug::{fold, slugify, unique_slug};
pub use vault::{
    ENV_VAR, INBOX_FILE, LOCK_FILE, META_DIR, PROJECT_FILE, TMP_SUFFIX, TOPICS_DIR, Vault,
};
#[cfg(feature = "watch")]
pub use watch::{DEFAULT_DEBOUNCE, VaultWatcher, is_ignored};

/// Today's date in local time (convenience for [`Snapshot::today`]).
pub fn local_today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

/// Re-export so consumers use the same `chrono` version for dates.
pub use chrono;
