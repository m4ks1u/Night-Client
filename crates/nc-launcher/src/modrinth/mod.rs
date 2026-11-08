//! Modrinth integration: API client, install planning and mods-folder
//! operations, and Modrinth modpack (.mrpack) installs. CurseForge is out of
//! scope for the beta.

pub mod api;
pub mod modpack;
pub mod mods;
pub mod packupdate;
pub mod plan;

pub use api::{Modrinth, ModrinthError};
