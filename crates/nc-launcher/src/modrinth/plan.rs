//! Install planning: which versions to download for a mod and its required
//! dependencies, and which problems to show before anything is written.
//!
//! Only *declared* metadata is checked (required dependencies and declared
//! incompatibilities). Many real conflicts cannot be detected statically;
//! the UI says so.

use std::collections::HashSet;

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::api::{Modrinth, ModrinthError, Version};

/// Where versions come from (the real client, or a fake in tests).
pub trait VersionSource {
    fn version(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> impl Future<Output = Result<Version, ModrinthError>> + Send;
    fn project_versions(
        &self,
        project: &str,
        loader: &str,
        minecraft: &str,
        cancel: &CancellationToken,
    ) -> impl Future<Output = Result<Vec<Version>, ModrinthError>> + Send;
}

impl VersionSource for Modrinth {
    fn version(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> impl Future<Output = Result<Version, ModrinthError>> + Send {
        Modrinth::version(self, id, cancel)
    }
    fn project_versions(
        &self,
        project: &str,
        loader: &str,
        minecraft: &str,
        cancel: &CancellationToken,
    ) -> impl Future<Output = Result<Vec<Version>, ModrinthError>> + Send {
        Modrinth::project_versions(self, project, loader, minecraft, cancel)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlannedMod {
    pub version: Version,
    /// `None` for the mod the user picked; otherwise the project that requires it.
    pub required_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// Versions to install, the chosen mod first.
    pub install: Vec<PlannedMod>,
    /// Required projects already installed (satisfied).
    pub already_installed: Vec<String>,
    /// Required projects with no version for this loader/game version.
    pub missing: Vec<String>,
    /// Installed (or planned) projects declared incompatible.
    pub conflicts: Vec<String>,
}

/// Prefer releases, then betas, then alphas; Modrinth returns newest first.
pub fn pick_best(mut versions: Vec<Version>, loader: &str, minecraft: &str) -> Option<Version> {
    versions.retain(|v| v.supports(loader, minecraft) && v.primary_file().is_some());
    let rank = |t: &str| match t {
        "release" => 0,
        "beta" => 1,
        _ => 2,
    };
    let best = versions.iter().map(|v| rank(&v.version_type)).min()?;
    versions.into_iter().find(|v| rank(&v.version_type) == best)
}

const MAX_PLANNED: usize = 64;

/// Plan installing `root` into an instance that already has `installed`
/// projects (by project id).
pub async fn plan(
    source: &impl VersionSource,
    root: Version,
    loader: &str,
    minecraft: &str,
    installed: &HashSet<String>,
    cancel: &CancellationToken,
) -> Result<Plan, ModrinthError> {
    let mut plan = Plan {
        install: Vec::new(),
        already_installed: Vec::new(),
        missing: Vec::new(),
        conflicts: Vec::new(),
    };
    let mut seen: HashSet<String> = HashSet::from([root.project_id.clone()]);
    let mut queue = vec![PlannedMod {
        version: root,
        required_by: None,
    }];

    while let Some(item) = queue.pop() {
        if plan.install.len() >= MAX_PLANNED {
            break;
        }
        let owner = item.version.project_id.clone();
        for dep in &item.version.dependencies {
            // Resolve the dependency's project id (a pinned version may omit it).
            let pinned = match (&dep.version_id, dep.dependency_type.as_str()) {
                (Some(vid), "required") => Some(source.version(vid, cancel).await?),
                _ => None,
            };
            let Some(project) = dep
                .project_id
                .clone()
                .or_else(|| pinned.as_ref().map(|v| v.project_id.clone()))
            else {
                continue;
            };
            match dep.dependency_type.as_str() {
                "required" => {
                    if installed.contains(&project) {
                        if !plan.already_installed.contains(&project) {
                            plan.already_installed.push(project);
                        }
                        continue;
                    }
                    if !seen.insert(project.clone()) {
                        continue;
                    }
                    let chosen = match pinned.filter(|v| v.supports(loader, minecraft)) {
                        Some(v) => Some(v),
                        None => pick_best(
                            source
                                .project_versions(&project, loader, minecraft, cancel)
                                .await?,
                            loader,
                            minecraft,
                        ),
                    };
                    match chosen {
                        Some(v) => queue.push(PlannedMod {
                            version: v,
                            required_by: Some(owner.clone()),
                        }),
                        None => plan.missing.push(project),
                    }
                }
                "incompatible"
                    if (installed.contains(&project) || seen.contains(&project))
                        && !plan.conflicts.contains(&project) =>
                {
                    plan.conflicts.push(project);
                }
                // optional / embedded: nothing to install.
                _ => {}
            }
        }
        plan.install.push(item);
    }
    // Chosen mod first, dependencies after, in discovery order.
    plan.install.sort_by_key(|p| p.required_by.is_some());
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::modrinth::api::{Dependency, Hashes, VersionFile};

    struct Fake {
        versions: HashMap<String, Version>,
    }

    impl VersionSource for Fake {
        async fn version(&self, id: &str, _: &CancellationToken) -> Result<Version, ModrinthError> {
            self.versions
                .get(id)
                .cloned()
                .ok_or(ModrinthError::NotFound)
        }
        async fn project_versions(
            &self,
            project: &str,
            loader: &str,
            mc: &str,
            _: &CancellationToken,
        ) -> Result<Vec<Version>, ModrinthError> {
            Ok(self
                .versions
                .values()
                .filter(|v| v.project_id == project && v.supports(loader, mc))
                .cloned()
                .collect())
        }
    }

    fn v(id: &str, project: &str, kind: &str, deps: &[(&str, &str)]) -> Version {
        Version {
            id: id.into(),
            project_id: project.into(),
            name: id.into(),
            version_number: "1".into(),
            version_type: kind.into(),
            loaders: vec!["fabric".into()],
            game_versions: vec!["1.21.1".into()],
            date_published: String::new(),
            files: vec![VersionFile {
                hashes: Hashes {
                    sha1: "a".into(),
                    sha512: "b".into(),
                },
                url: "https://cdn.modrinth.com/x.jar".into(),
                filename: format!("{id}.jar"),
                primary: true,
                size: 1,
            }],
            dependencies: deps
                .iter()
                .map(|(p, t)| Dependency {
                    version_id: None,
                    project_id: Some((*p).into()),
                    file_name: None,
                    dependency_type: (*t).into(),
                })
                .collect(),
        }
    }

    fn fake(list: Vec<Version>) -> Fake {
        Fake {
            versions: list.into_iter().map(|v| (v.id.clone(), v)).collect(),
        }
    }

    #[tokio::test]
    async fn resolves_transitive_required_dependencies_once() {
        let src = fake(vec![
            v("lib-b", "B", "release", &[("C", "required")]),
            v("lib-c", "C", "release", &[("B", "required")]), // cycle
            v("opt-d", "D", "release", &[]),
        ]);
        let root = v(
            "root",
            "A",
            "release",
            &[("B", "required"), ("D", "optional")],
        );
        let p = plan(
            &src,
            root,
            "fabric",
            "1.21.1",
            &HashSet::new(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let ids: Vec<_> = p.install.iter().map(|m| m.version.id.as_str()).collect();
        assert_eq!(ids[0], "root");
        assert_eq!(
            ids.len(),
            3,
            "A, B, C once each; optional D skipped: {ids:?}"
        );
        assert!(p.missing.is_empty() && p.conflicts.is_empty());
    }

    #[tokio::test]
    async fn reports_installed_missing_and_conflicts() {
        let src = fake(vec![v("lib-b", "B", "release", &[])]);
        let root = v(
            "root",
            "A",
            "release",
            &[
                ("B", "required"),
                ("NOPE", "required"),
                ("X", "incompatible"),
            ],
        );
        let installed = HashSet::from(["B".to_owned(), "X".to_owned()]);
        let p = plan(
            &src,
            root,
            "fabric",
            "1.21.1",
            &installed,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(p.install.len(), 1);
        assert_eq!(p.already_installed, ["B"]);
        assert_eq!(p.missing, ["NOPE"]);
        assert_eq!(p.conflicts, ["X"]);
    }

    #[test]
    fn prefers_releases_and_compatible_files() {
        let mut beta = v("beta", "A", "beta", &[]);
        beta.date_published = "2026".into();
        let release = v("rel", "A", "release", &[]);
        let mut wrong = v("neo", "A", "release", &[]);
        wrong.loaders = vec!["neoforge".into()];
        assert_eq!(
            pick_best(
                vec![wrong.clone(), beta.clone(), release],
                "fabric",
                "1.21.1"
            )
            .unwrap()
            .id,
            "rel"
        );
        assert_eq!(
            pick_best(vec![beta, wrong], "fabric", "1.21.1").unwrap().id,
            "beta"
        );
    }

    #[test]
    fn quilt_instances_take_fabric_mods_but_fabric_takes_no_quilt_mods() {
        let fabric = v("fab", "A", "release", &[]);
        let mut quilt = v("qui", "A", "release", &[]);
        quilt.loaders = vec!["quilt".into()];
        assert_eq!(
            pick_best(vec![fabric.clone()], "quilt", "1.21.1")
                .unwrap()
                .id,
            "fab"
        );
        assert!(pick_best(vec![quilt.clone()], "fabric", "1.21.1").is_none());
        assert!(pick_best(vec![fabric, quilt], "neoforge", "1.21.1").is_none());
    }
}
