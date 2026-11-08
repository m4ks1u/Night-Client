//! Parsing and validation of user-supplied JVM arguments.
//!
//! Arguments are split without a shell and checked against an allowlist.
//! Anything that could run commands (`-XX:OnError`), load code from outside
//! the game (`-javaagent`, `-cp`, `@argfiles`), or override paths the
//! launcher controls (natives, logging config, classpath) is refused.
//! Memory is set with the launcher's slider, not `-Xmx`.

use thiserror::Error;

pub const MAX_ARGS: usize = 64;
pub const MAX_ARG_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JvmArgError {
    #[error("unbalanced quote")]
    UnbalancedQuote,
    #[error("too many arguments (at most {MAX_ARGS})")]
    TooMany,
    #[error("`{0}` is too long")]
    TooLong(String),
    #[error("`{0}` contains control characters")]
    Control(String),
    #[error("`{0}` is not a JVM option (options start with -)")]
    NotAnOption(String),
    #[error("`{0}`: set memory with the memory slider instead")]
    Memory(String),
    #[error("`{arg}` is not allowed: {reason}")]
    Blocked { arg: String, reason: &'static str },
    #[error(
        "`{0}` is not supported; allowed: -XX:…, -D…, -Xss, -Xmn, -Xlog, --add-opens/--add-exports/--add-reads, --enable-native-access, -ea/-da"
    )]
    NotAllowed(String),
    #[error("`{0}` needs a value")]
    MissingValue(String),
}

/// Split like a command line: whitespace separates, double quotes group,
/// `\"` inside quotes is a literal quote. No variables, globbing or escapes
/// outside quotes — there is no shell.
pub fn split(input: &str) -> Result<Vec<String>, JvmArgError> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            '\\' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if in_quotes {
        return Err(JvmArgError::UnbalancedQuote);
    }
    if has_token {
        out.push(cur);
    }
    Ok(out)
}

/// System properties the launcher sets or that change where code/natives
/// are loaded from.
const BLOCKED_PROPERTIES: &[&str] = &[
    "java.library.path",
    "java.class.path",
    "java.home",
    "java.system.class.loader",
    "java.security.manager",
    "java.security.policy",
    "jdk.module.path",
    "jdk.module.upgrade.path",
    "jna.library.path",
    "jna.boot.library.path",
    "jna.tmpdir",
    "org.lwjgl.librarypath",
    "org.lwjgl.system.SharedLibraryExtractPath",
    "io.netty.native.workdir",
    "log4j.configurationFile",
    "log4j2.configurationFile",
    "minecraft.launcher.brand",
    "minecraft.launcher.version",
];

/// `-XX` flags that execute commands or read further options from files.
const BLOCKED_XX: &[&str] = &[
    "OnError",
    "OnOutOfMemoryError",
    "Flags",
    "VMOptionsFile",
    "+EnableDynamicAgentLoading",
];

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn is_size(s: &str) -> bool {
    let digits = s.trim_end_matches(['k', 'K', 'm', 'M', 'g', 'G']);
    !digits.is_empty() && digits.len() <= 12 && digits.chars().all(|c| c.is_ascii_digit())
}

/// `module/package=target(,target)*` or `ALL-UNNAMED` targets.
fn is_module_spec(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '=' | ','))
}

pub fn validate(args: &[String]) -> Result<(), JvmArgError> {
    if args.len() > MAX_ARGS {
        return Err(JvmArgError::TooMany);
    }
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a.len() > MAX_ARG_LEN {
            return Err(JvmArgError::TooLong(a.chars().take(40).collect()));
        }
        if a.chars().any(char::is_control) {
            return Err(JvmArgError::Control(a.escape_debug().to_string()));
        }
        let blocked = |reason| {
            Err(JvmArgError::Blocked {
                arg: a.to_owned(),
                reason,
            })
        };

        if a.starts_with('@') {
            return blocked("argument files can load arbitrary options");
        }
        if !a.starts_with('-') {
            return Err(JvmArgError::NotAnOption(a.to_owned()));
        }
        if matches!(
            a,
            "-cp"
                | "-classpath"
                | "--class-path"
                | "-jar"
                | "-p"
                | "--module-path"
                | "--upgrade-module-path"
                | "--patch-module"
        ) || a.starts_with("--class-path=")
            || a.starts_with("--module-path=")
            || a.starts_with("--patch-module=")
        {
            return blocked("the launcher builds the classpath");
        }
        if a.starts_with("-javaagent") || a.starts_with("-agentlib") || a.starts_with("-agentpath")
        {
            return blocked("Java agents run code outside the game");
        }
        if a.starts_with("-Xbootclasspath") {
            return blocked("changes how the JVM loads core classes");
        }
        if a.starts_with("-Xmx")
            || a.starts_with("-Xms")
            || a.starts_with("-XX:MaxRAM")
            || a.starts_with("-XX:MaxHeapSize")
            || a.starts_with("-XX:InitialHeapSize")
        {
            return Err(JvmArgError::Memory(a.to_owned()));
        }

        if let Some(rest) = a.strip_prefix("-XX:") {
            let flag = rest.trim_start_matches(['+', '-']);
            let name = flag.split('=').next().unwrap_or("");
            if BLOCKED_XX.iter().any(|b| rest.starts_with(b) || name == *b) {
                return blocked("this flag can run commands or read options from files");
            }
            let value_ok = flag
                .split_once('=')
                .is_none_or(|(_, v)| !v.is_empty() && v.len() <= 256 && !v.contains(['"', '\'']));
            if !is_name(name) || !value_ok {
                return Err(JvmArgError::NotAllowed(a.to_owned()));
            }
        } else if let Some(prop) = a.strip_prefix("-D") {
            let key = prop.split('=').next().unwrap_or("");
            if !is_name(key) {
                return Err(JvmArgError::NotAllowed(a.to_owned()));
            }
            if BLOCKED_PROPERTIES
                .iter()
                .any(|b| b.eq_ignore_ascii_case(key))
            {
                return blocked("the launcher controls this property");
            }
        } else if let Some(v) = a.strip_prefix("-Xss").or_else(|| a.strip_prefix("-Xmn")) {
            if !is_size(v) {
                return Err(JvmArgError::NotAllowed(a.to_owned()));
            }
        } else if a.starts_with("-Xlog:") || matches!(a, "-ea" | "-da" | "-esa" | "-dsa") {
            // Diagnostic logging and assertions are harmless.
        } else if let Some(opt) = [
            "--add-opens",
            "--add-exports",
            "--add-reads",
            "--add-modules",
            "--enable-native-access",
        ]
        .into_iter()
        .find(|o| a == *o || a.starts_with(&format!("{o}=")))
        {
            let value = if a == opt {
                i += 1;
                args.get(i)
                    .map(String::as_str)
                    .ok_or_else(|| JvmArgError::MissingValue(a.to_owned()))?
            } else {
                &a[opt.len() + 1..]
            };
            if !is_module_spec(value) {
                return Err(JvmArgError::NotAllowed(format!("{opt} {value}")));
            }
        } else {
            return Err(JvmArgError::NotAllowed(a.to_owned()));
        }
        i += 1;
    }
    Ok(())
}

/// Split and validate a user-entered argument string.
pub fn parse(input: &str) -> Result<Vec<String>, JvmArgError> {
    let args = split(input)?;
    validate(&args)?;
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_a_command_line_without_a_shell() {
        assert_eq!(
            split("  -XX:+UseG1GC   -Dfoo=bar ").unwrap(),
            ["-XX:+UseG1GC", "-Dfoo=bar"]
        );
        assert_eq!(
            split(r#"-Dname="hello world" -Dq="a\"b""#).unwrap(),
            ["-Dname=hello world", r#"-Dq=a"b"#]
        );
        assert_eq!(
            split("-Dx=$(calc) %PATH% `id`").unwrap(),
            ["-Dx=$(calc)", "%PATH%", "`id`"],
            "no expansion"
        );
        assert_eq!(split("").unwrap(), Vec::<String>::new());
        assert_eq!(split(r#""""#).unwrap(), [""], "empty quoted token is kept");
        assert_eq!(split(r#"-Da="x"#), Err(JvmArgError::UnbalancedQuote));
    }

    #[test]
    fn accepts_common_tuning_flags() {
        let ok = "-XX:+UseG1GC -XX:+UnlockExperimentalVMOptions -XX:G1NewSizePercent=20 -XX:MaxGCPauseMillis=50 \
                  -XX:-UseAdaptiveSizePolicy -Xss2M -Xmn512m -Dfile.encoding=UTF-8 -Dfml.ignoreInvalidMinecraftCertificates=true \
                  --add-opens java.base/java.lang=ALL-UNNAMED --add-exports=java.base/sun.nio.ch=ALL-UNNAMED \
                  --enable-native-access=ALL-UNNAMED -Xlog:gc*:file=gc.log -ea";
        assert!(parse(ok).is_ok(), "{:?}", parse(ok));
    }

    #[test]
    fn rejects_code_loading_and_command_execution() {
        for bad in [
            "-javaagent:C:\\evil.jar",
            "-agentlib:jdwp=transport=dt_socket",
            "-agentpath:C:\\evil.dll",
            "-cp C:\\evil",
            "-jar evil.jar",
            "--class-path=C:\\x",
            "--module-path x",
            "-Xbootclasspath/a:C:\\evil.jar",
            "-XX:OnError=calc.exe",
            r#"-XX:OnOutOfMemoryError="cmd /c del""#,
            "-XX:Flags=C:\\flags.txt",
            "-XX:VMOptionsFile=C:\\x",
            "@C:\\args.txt",
            "-Djava.library.path=C:\\evil",
            "-DJAVA.LIBRARY.PATH=C:\\evil",
            "-Dlog4j.configurationFile=http://evil/x.xml",
            "-Dorg.lwjgl.librarypath=C:\\evil",
            "-Djava.system.class.loader=evil.Loader",
        ] {
            assert!(
                matches!(parse(bad), Err(JvmArgError::Blocked { .. })),
                "{bad}: {:?}",
                parse(bad)
            );
        }
    }

    #[test]
    fn rejects_other_invalid_input() {
        assert!(matches!(parse("-Xmx8G"), Err(JvmArgError::Memory(_))));
        assert!(matches!(
            parse("-XX:MaxRAMPercentage=90"),
            Err(JvmArgError::Memory(_))
        ));
        assert!(matches!(parse("UseG1GC"), Err(JvmArgError::NotAnOption(_))));
        assert!(matches!(
            parse("-verbose:class"),
            Err(JvmArgError::NotAllowed(_))
        ));
        assert!(matches!(parse("-Xss2Q"), Err(JvmArgError::NotAllowed(_))));
        assert!(matches!(parse("-D=x"), Err(JvmArgError::NotAllowed(_))));
        assert!(matches!(
            parse("-Dkey with space=1"),
            Err(JvmArgError::NotAnOption(_)) | Err(JvmArgError::NotAllowed(_))
        ));
        assert!(matches!(
            parse("--add-opens"),
            Err(JvmArgError::MissingValue(_))
        ));
        assert!(matches!(
            parse("--add-opens \"java.base/x=y;calc\""),
            Err(JvmArgError::NotAllowed(_))
        ));
        assert!(matches!(
            validate(&["-Dx=\u{0}".into()]),
            Err(JvmArgError::Control(_))
        ));
        assert!(matches!(
            validate(&[format!("-Dx={}", "a".repeat(600))]),
            Err(JvmArgError::TooLong(_))
        ));
        let many: Vec<String> = (0..65).map(|i| format!("-Dk{i}=v")).collect();
        assert_eq!(validate(&many), Err(JvmArgError::TooMany));
    }
}
