//! Resolution of `inheritsFrom` chains (Fabric, NeoForge, Quilt profiles).

use std::collections::HashSet;

use crate::{CoreError, manifest::VersionJson, maven::MavenCoord};

/// Merge a child profile onto its parent.
///
/// * Scalars: the child wins when present.
/// * Arguments: parent first, then child (loaders append).
/// * Libraries: child first; a parent library with the same
///   `group:artifact[:classifier]` as a child library is dropped, so loaders
///   can override vanilla library versions.
pub fn merge(child: VersionJson, parent: VersionJson) -> Result<VersionJson, CoreError> {
    if let Some(expected) = &child.inherits_from
        && expected != &parent.id
    {
        return Err(CoreError::ParentMismatch {
            child: child.id.clone(),
            expected: expected.clone(),
            actual: parent.id.clone(),
        });
    }

    let child_keys: HashSet<String> = child
        .libraries
        .iter()
        .filter_map(|l| MavenCoord::parse(&l.name).ok().map(|c| c.key()))
        .collect();

    let mut libraries = child.libraries;
    libraries.extend(parent.libraries.into_iter().filter(|l| {
        MavenCoord::parse(&l.name)
            .map(|c| !child_keys.contains(&c.key()))
            .unwrap_or(true)
    }));

    let arguments = match (parent.arguments, child.arguments) {
        (Some(mut p), Some(c)) => {
            p.game.extend(c.game);
            p.jvm.extend(c.jvm);
            Some(p)
        }
        (p, c) => p.or(c),
    };

    Ok(VersionJson {
        id: child.id,
        inherits_from: parent.inherits_from,
        kind: child.kind.or(parent.kind),
        main_class: child.main_class.or(parent.main_class),
        arguments,
        minecraft_arguments: child.minecraft_arguments.or(parent.minecraft_arguments),
        asset_index: child.asset_index.or(parent.asset_index),
        assets: child.assets.or(parent.assets),
        downloads: child.downloads.or(parent.downloads),
        libraries,
        java_version: child.java_version.or(parent.java_version),
        logging: child.logging.or(parent.logging),
    })
}
