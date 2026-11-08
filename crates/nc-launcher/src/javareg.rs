//! Java homes recorded in the Windows registry by vendor installers.
//!
//! Only string values that name an install folder are read (`JavaHome`,
//! `Path`, `InstallationPath`); nothing is written or executed here. Every
//! result still goes through `javadetect::inspect`, which checks it is a real
//! Java home. Network (UNC) paths are ignored, as for custom Java paths.

use std::path::PathBuf;

use windows_registry::{CURRENT_USER, Key, LOCAL_MACHINE};

/// Vendor roots under `SOFTWARE`, as written by the common installers:
/// Oracle and older OpenJDK builds (`JavaSoft\JDK\<ver>\JavaHome`), Adoptium
/// and Microsoft (`…\<ver>\hotspot\MSI\Path`), Azul, BellSoft, Corretto.
pub const VENDOR_ROOTS: &[&str] = &[
    r"SOFTWARE\JavaSoft",
    r"SOFTWARE\Eclipse Adoptium",
    r"SOFTWARE\Eclipse Foundation",
    r"SOFTWARE\AdoptOpenJDK",
    r"SOFTWARE\Microsoft\JDK",
    r"SOFTWARE\Azul Systems\Zulu",
    r"SOFTWARE\BellSoft\Liberica",
    r"SOFTWARE\Amazon Corretto",
    r"SOFTWARE\Semeru",
];

const VALUE_NAMES: &[&str] = &["JavaHome", "Path", "InstallationPath"];
/// Deep enough for `JDK\<ver>\hotspot\MSI`; bounds the walk.
const MAX_DEPTH: usize = 4;
/// Bounds the walk on a registry with an unusual number of entries.
const MAX_KEYS: usize = 512;

/// Candidate homes from HKLM and HKCU under each root in `roots`.
pub fn homes(roots: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut budget = MAX_KEYS;
    for hive in [LOCAL_MACHINE, CURRENT_USER] {
        for root in roots {
            if let Ok(key) = hive.open(root) {
                walk(&key, MAX_DEPTH, &mut budget, &mut out);
            }
        }
    }
    out
}

fn walk(key: &Key, depth: usize, budget: &mut usize, out: &mut Vec<PathBuf>) {
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    for name in VALUE_NAMES {
        if let Ok(v) = key.get_string(name)
            && let Some(p) = usable(&v)
            && !out.contains(&p)
        {
            out.push(p);
        }
    }
    if depth == 0 {
        return;
    }
    let Ok(names) = key.keys() else {
        return;
    };
    for sub in names.take(64) {
        if let Ok(child) = key.open(&sub) {
            walk(&child, depth - 1, budget, out);
        }
    }
}

/// A local, absolute folder path, or `None`.
fn usable(v: &str) -> Option<PathBuf> {
    let v = v.trim().trim_end_matches(['\\', '/']);
    let local = v.len() >= 3
        && v.len() <= 1024
        && v.as_bytes()[0].is_ascii_alphabetic()
        && v[1..].starts_with(r":\");
    local.then(|| PathBuf::from(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_absolute_paths_are_used() {
        assert_eq!(
            usable(r"C:\Program Files\Java\jdk-21\"),
            Some(PathBuf::from(r"C:\Program Files\Java\jdk-21"))
        );
        for bad in [
            r"\\server\share\jdk",
            "//server/share/jdk",
            r"jdk\bin",
            r"C:",
            "",
            r"\\?\C:\jdk",
        ] {
            assert_eq!(usable(bad), None, "{bad}");
        }
    }

    #[test]
    fn walks_vendor_layouts_in_the_registry() {
        // A private HKCU tree shaped like the Oracle and Microsoft installers.
        let base = format!(r"Software\NightClientTest-javareg-{}", std::process::id());
        let d = tempfile::tempdir().unwrap();
        let oracle = d.path().join("jdk-17");
        let ms = d.path().join("ms-21");
        let k = CURRENT_USER
            .create(format!(r"{base}\JavaSoft\JDK\17.0.9"))
            .unwrap();
        k.set_string("JavaHome", oracle.to_str().unwrap()).unwrap();
        let k = CURRENT_USER
            .create(format!(r"{base}\Microsoft\JDK\21.0.7.6\hotspot\MSI"))
            .unwrap();
        k.set_string("Path", format!(r"{}\", ms.display())).unwrap();
        let k = CURRENT_USER.create(format!(r"{base}\Evil\1")).unwrap();
        k.set_string("JavaHome", r"\\attacker\share\jdk").unwrap();

        let found = homes(&[
            &format!(r"{base}\JavaSoft"),
            &format!(r"{base}\Microsoft\JDK"),
            &format!(r"{base}\Evil"),
        ]);
        CURRENT_USER.remove_tree(&base).unwrap();
        assert_eq!(found, [oracle, ms]);
    }
}
