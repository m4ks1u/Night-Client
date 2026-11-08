//! Night Client core: a pure (I/O-free) model of Minecraft Java Edition metadata.
//!
//! This crate parses the official version manifest and version JSON files,
//! evaluates platform rules, resolves libraries and builds launch commands.
//! It never touches the network or the file system, which keeps it fast to
//! test and safe to reuse from other front ends.

#![forbid(unsafe_code)]

pub mod content;
pub mod error;
pub mod installprofile;
pub mod java;
pub mod jvmargs;
pub mod launch;
pub mod libraries;
pub mod loader;
pub mod manifest;
pub mod maven;
pub mod paths;
pub mod profile;
pub mod rules;
pub mod secret;

pub use error::CoreError;
pub use secret::Secret;
