//! Construction of the Java command line for a resolved version.

use std::{collections::HashMap, fmt, path::PathBuf};

use crate::{
    CoreError, Secret,
    libraries::{LibraryKind, ResolvedLibrary},
    manifest::{Argument, VersionJson},
    rules::{Environment, OsName, rules_allow},
};

#[derive(Debug, Clone)]
pub struct LaunchPaths {
    pub java: PathBuf,
    pub game_dir: PathBuf,
    pub assets_root: PathBuf,
    pub libraries_dir: PathBuf,
    pub natives_dir: PathBuf,
    pub client_jar: PathBuf,
    /// Downloaded log4j configuration, if the version declares one.
    pub log_config: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct LaunchAuth {
    pub player_name: String,
    /// Profile UUID without dashes.
    pub uuid: String,
    pub access_token: Secret,
    pub xuid: Option<String>,
    pub client_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub min_memory_mb: Option<u32>,
    pub max_memory_mb: u32,
    pub extra_jvm_args: Vec<String>,
    pub resolution: Option<(u32, u32)>,
    pub launcher_name: String,
    pub launcher_version: String,
}

/// A ready-to-spawn command. Arguments are passed as a vector to the OS,
/// never through a shell.
#[derive(Clone)]
pub struct LaunchCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    secrets: Vec<Secret>,
}

impl LaunchCommand {
    /// Arguments with every secret replaced, safe for logs and diagnostics.
    pub fn redacted_args(&self) -> Vec<String> {
        let refs: Vec<&Secret> = self.secrets.iter().collect();
        self.args
            .iter()
            .map(|a| crate::secret::redact(a, &refs))
            .collect()
    }

    pub fn secrets(&self) -> &[Secret] {
        &self.secrets
    }
}

impl fmt::Debug for LaunchCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchCommand")
            .field("program", &self.program)
            .field("args", &self.redacted_args())
            .field("cwd", &self.cwd)
            .finish()
    }
}

/// Classpath in launch order: libraries, then the client jar.
pub fn classpath(libraries: &[ResolvedLibrary], paths: &LaunchPaths) -> Vec<PathBuf> {
    let mut cp: Vec<PathBuf> = libraries
        .iter()
        .filter(|l| l.kind == LibraryKind::Classpath)
        .map(|l| {
            paths
                .libraries_dir
                .join(l.path.replace('/', std::path::MAIN_SEPARATOR_STR))
        })
        .collect();
    cp.push(paths.client_jar.clone());
    cp
}

pub fn build(
    version: &VersionJson,
    env: &Environment,
    libraries: &[ResolvedLibrary],
    paths: &LaunchPaths,
    auth: &LaunchAuth,
    opts: &LaunchOptions,
) -> Result<LaunchCommand, CoreError> {
    let main_class = version
        .main_class
        .clone()
        .ok_or_else(|| CoreError::MissingField {
            version: version.id.clone(),
            field: "mainClass",
        })?;
    let sep = if env.os == OsName::Windows { ";" } else { ":" };
    let cp = classpath(libraries, paths)
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(sep);

    let mut env = env.clone();
    env.features
        .insert("has_custom_resolution".into(), opts.resolution.is_some());

    let assets_index_name = version
        .asset_index
        .as_ref()
        .map(|a| a.id.clone())
        .or_else(|| version.assets.clone())
        .unwrap_or_else(|| "legacy".into());

    let d = |p: &PathBuf| p.display().to_string();
    let (w, h) = opts.resolution.unwrap_or((854, 480));
    let vars: HashMap<&str, String> = HashMap::from([
        ("auth_player_name", auth.player_name.clone()),
        ("version_name", version.id.clone()),
        ("game_directory", d(&paths.game_dir)),
        ("assets_root", d(&paths.assets_root)),
        (
            "game_assets",
            d(&paths.assets_root.join("virtual").join("legacy")),
        ),
        ("assets_index_name", assets_index_name),
        ("auth_uuid", auth.uuid.clone()),
        ("auth_access_token", auth.access_token.expose().to_owned()),
        (
            "auth_session",
            format!("token:{}:{}", auth.access_token.expose(), auth.uuid),
        ),
        ("clientid", auth.client_id.clone().unwrap_or_default()),
        ("auth_xuid", auth.xuid.clone().unwrap_or_else(|| "0".into())),
        ("user_type", "msa".into()),
        (
            "version_type",
            version.kind.map(|k| k.as_str()).unwrap_or("release").into(),
        ),
        ("user_properties", "{}".into()),
        ("natives_directory", d(&paths.natives_dir)),
        ("library_directory", d(&paths.libraries_dir)),
        ("classpath_separator", sep.into()),
        ("launcher_name", opts.launcher_name.clone()),
        ("launcher_version", opts.launcher_version.clone()),
        ("classpath", cp),
        ("resolution_width", w.to_string()),
        ("resolution_height", h.to_string()),
    ]);

    let mut args = Vec::new();
    if let Some(min) = opts.min_memory_mb {
        args.push(format!("-Xms{min}M"));
    }
    args.push(format!("-Xmx{}M", opts.max_memory_mb));
    args.extend(opts.extra_jvm_args.iter().cloned());

    match &version.arguments {
        Some(a) => push_args(&mut args, &a.jvm, &env, &vars),
        None => {
            // Pre-1.13 versions carry no JVM arguments; use the launcher defaults.
            let mut legacy = Vec::new();
            if env.os == OsName::Windows {
                legacy.push(
                    "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump",
                );
            }
            legacy.extend([
                "-Djava.library.path=${natives_directory}",
                "-Dminecraft.launcher.brand=${launcher_name}",
                "-Dminecraft.launcher.version=${launcher_version}",
                "-cp",
                "${classpath}",
            ]);
            args.extend(legacy.into_iter().map(|s| substitute(s, &vars)));
        }
    }

    if let (Some(cfg), Some(path)) = (
        version.logging.as_ref().and_then(|l| l.client.as_ref()),
        &paths.log_config,
    ) {
        args.push(cfg.argument.replace("${path}", &path.display().to_string()));
    }

    args.push(main_class);

    match (&version.arguments, &version.minecraft_arguments) {
        (Some(a), _) if !a.game.is_empty() => push_args(&mut args, &a.game, &env, &vars),
        (_, Some(legacy)) => {
            args.extend(legacy.split_whitespace().map(|s| substitute(s, &vars)));
            if let Some((w, h)) = opts.resolution {
                args.extend([
                    "--width".into(),
                    w.to_string(),
                    "--height".into(),
                    h.to_string(),
                ]);
            }
        }
        _ => {}
    }

    Ok(LaunchCommand {
        program: paths.java.clone(),
        args,
        cwd: paths.game_dir.clone(),
        secrets: vec![auth.access_token.clone()],
    })
}

fn push_args(
    out: &mut Vec<String>,
    args: &[Argument],
    env: &Environment,
    vars: &HashMap<&str, String>,
) {
    for arg in args {
        match arg {
            Argument::Plain(s) => out.push(substitute(s, vars)),
            Argument::Conditional { rules, value } => {
                if rules_allow(rules, env) {
                    out.extend(value.iter().map(|s| substitute(s, vars)));
                }
            }
        }
    }
}

/// Replace `${name}` placeholders. Unknown placeholders are kept verbatim so
/// problems are visible in diagnostics rather than silently dropped.
pub fn substitute(template: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                match vars.get(key) {
                    Some(v) => out.push_str(v),
                    None => {
                        out.push_str("${");
                        out.push_str(key);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_handles_known_unknown_and_unterminated() {
        let vars = HashMap::from([("a", "1".to_string()), ("b", "${a}".to_string())]);
        assert_eq!(substitute("x${a}y${a}", &vars), "x1y1");
        assert_eq!(substitute("${nope}", &vars), "${nope}");
        assert_eq!(substitute("${a", &vars), "${a");
        // Values are not re-expanded (no injection via substituted values).
        assert_eq!(substitute("${b}", &vars), "${a}");
    }
}
