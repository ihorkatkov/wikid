//! wikid-core: vault model, link graph, file operations, and health checks.
//!
//! A wiki is any directory of Markdown files (Obsidian vaults included).
//! This crate holds no state that isn't derivable from the files themselves.
//! Every public result type derives `Serialize` and `Deserialize` — they are
//! the wire format shared by the CLI, HTTP, and MCP surfaces.

pub mod doctor;
pub mod error;
pub mod frontmatter;
pub mod links;
pub mod ops;
mod paths;
pub mod status;
pub mod tags;
pub mod vault;

#[cfg(test)]
pub mod test_fixtures;

pub use doctor::{Check, DoctorOptions, DoctorProfile, HealthReport, Issue, IssueCategory, Severity, SeveritySummary};
pub use error::WikidError;
pub use frontmatter::Frontmatter;
pub use links::{Link, LinkKind, LinkReport};
pub use ops::{
	Document, EditResult, Entry, EntryKind, GlobResult, GrepMatch, GrepOptions, GrepResult, Hashline, HashlinesResult,
	LineEdit, Listing, MvResult, ReadLimit, ReadRange, RmResult, WriteResult, hash_line,
};
pub use status::{RecentPage, VaultStatus};
pub use tags::{TagReport, TagSummary};
pub use vault::Vault;
