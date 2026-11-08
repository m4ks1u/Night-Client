//! Modrinth integration: API client, install planning and mods-folder
//! operations. CurseForge and modpack sync are out of scope for the beta.

pub mod api;
pub mod mods;
pub mod plan;

pub use api::{Modrinth, ModrinthError};
