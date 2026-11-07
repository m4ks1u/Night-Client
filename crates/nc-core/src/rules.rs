use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub action: RuleAction,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<BTreeMap<String, bool>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsName {
    Windows,
    Linux,
    Osx,
}

impl OsName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Osx => "osx",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    X86,
    Aarch64,
}

impl Arch {
    fn matches(self, rule: &str) -> bool {
        match rule {
            "x86" => self == Self::X86,
            "x86_64" | "amd64" | "x64" => self == Self::X86_64,
            "arm64" | "aarch64" => self == Self::Aarch64,
            _ => false,
        }
    }

    /// Value substituted for `${arch}` in legacy native classifiers.
    pub fn bits(self) -> &'static str {
        match self {
            Self::X86 => "32",
            _ => "64",
        }
    }
}

/// The platform and feature flags rules are evaluated against.
#[derive(Debug, Clone)]
pub struct Environment {
    pub os: OsName,
    pub arch: Arch,
    /// e.g. `10.0` on Windows 10/11 (Java reports `os.version` this way).
    pub os_version: String,
    pub features: BTreeMap<String, bool>,
}

impl Environment {
    pub fn windows_x64() -> Self {
        Self {
            os: OsName::Windows,
            arch: Arch::X86_64,
            os_version: "10.0".into(),
            features: BTreeMap::new(),
        }
    }

    pub fn with_feature(mut self, name: &str, on: bool) -> Self {
        self.features.insert(name.to_owned(), on);
        self
    }
}

/// Mojang semantics: no rules means allowed; otherwise start disallowed and
/// let every *matching* rule set the outcome, last match wins.
pub fn rules_allow(rules: &[Rule], env: &Environment) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule_matches(rule, env) {
            allowed = rule.action == RuleAction::Allow;
        }
    }
    allowed
}

fn rule_matches(rule: &Rule, env: &Environment) -> bool {
    if let Some(os) = &rule.os {
        if os.name.as_deref().is_some_and(|n| n != env.os.as_str()) {
            return false;
        }
        if os.arch.as_deref().is_some_and(|a| !env.arch.matches(a)) {
            return false;
        }
        if let Some(pattern) = &os.version {
            // An invalid regex from metadata must not match (fail closed).
            let ok = Regex::new(pattern).is_ok_and(|re| re.is_match(&env.os_version));
            if !ok {
                return false;
            }
        }
    }
    if let Some(features) = &rule.features {
        for (name, wanted) in features {
            if env.features.get(name).copied().unwrap_or(false) != *wanted {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Vec<Rule> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn empty_rules_allow() {
        assert!(rules_allow(&[], &Environment::windows_x64()));
    }

    #[test]
    fn os_allow_and_disallow() {
        let env = Environment::windows_x64();
        assert!(rules_allow(
            &parse(r#"[{"action":"allow","os":{"name":"windows"}}]"#),
            &env
        ));
        assert!(!rules_allow(
            &parse(r#"[{"action":"allow","os":{"name":"osx"}}]"#),
            &env
        ));
        assert!(rules_allow(
            &parse(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#),
            &env
        ));
        assert!(!rules_allow(
            &parse(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"windows"}}]"#),
            &env
        ));
    }

    #[test]
    fn arch_x86_means_32_bit() {
        let rules = parse(r#"[{"action":"allow","os":{"arch":"x86"}}]"#);
        assert!(!rules_allow(&rules, &Environment::windows_x64()));
        let mut env32 = Environment::windows_x64();
        env32.arch = Arch::X86;
        assert!(rules_allow(&rules, &env32));
    }

    #[test]
    fn os_version_regex_and_invalid_regex() {
        let env = Environment::windows_x64();
        assert!(rules_allow(
            &parse(r#"[{"action":"allow","os":{"name":"windows","version":"^10\\."}}]"#),
            &env
        ));
        assert!(!rules_allow(
            &parse(r#"[{"action":"allow","os":{"version":"(unclosed"}}]"#),
            &env
        ));
    }

    #[test]
    fn features_must_all_match() {
        let rules = parse(r#"[{"action":"allow","features":{"is_demo_user":true}}]"#);
        assert!(!rules_allow(&rules, &Environment::windows_x64()));
        assert!(rules_allow(
            &rules,
            &Environment::windows_x64().with_feature("is_demo_user", true)
        ));
    }
}
