//! wikid-core: vault model, link graph, file operations, and health checks.
//!
//! A wiki is any directory of Markdown files (Obsidian vaults included).
//! This crate holds no state that isn't derivable from the files themselves.

pub mod vault;

pub use vault::Vault;
