//! Turning packs on in the game's own settings, without touching anything
//! else in those files.
//!
//! - Resource packs: `options.txt`, line `resourcePacks:[...]` (a JSON array
//!   of pack ids, lowest priority first). Since Minecraft 1.13 a pack file's
//!   id is `file/<name>`; before that it is just `<name>`.
//! - Shader packs (Iris): `config/iris.properties`, keys `shaderPack` and
//!   `enableShaders` (Java properties, ISO-8859-1 with `\uXXXX` escapes).
//!
//! Every other line is kept byte for byte, in order. Line endings follow the
//! file (CRLF if it uses CRLF).

/// Whether resource pack ids carry the `file/` prefix (Minecraft 1.13+,
/// snapshots from 2018 on, and year-based versions).
pub fn uses_file_prefix(minecraft: &str) -> bool {
    let mut parts = minecraft.split('.');
    match (parts.next(), parts.next()) {
        (Some("1"), Some(minor)) => minor
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|m| m.parse::<u32>().ok())
            .is_some_and(|m| m >= 13),
        (Some(year), Some(_)) => year.len() == 2 && year.parse::<u32>().is_ok_and(|y| y >= 25),
        // Snapshots: "24w14a".
        (Some(s), None) => s
            .split_once('w')
            .and_then(|(y, _)| y.parse::<u32>().ok())
            .is_some_and(|y| y >= 18),
        _ => false,
    }
}

/// The id `options.txt` uses for a pack file.
pub fn pack_id(file_name: &str, file_prefix: bool) -> String {
    if file_prefix {
        format!("file/{file_name}")
    } else {
        file_name.to_owned()
    }
}

fn eol(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
}

/// The `resourcePacks` list, if the line is there and parses.
pub fn resource_packs(options: &str) -> Option<Vec<String>> {
    lines(options)
        .find_map(|l| l.strip_prefix("resourcePacks:"))
        .and_then(|v| serde_json::from_str(v.trim()).ok())
}

/// Replace (or add) the `resourcePacks` line.
pub fn set_resource_packs(options: &str, packs: &[String]) -> String {
    let value = format!(
        "resourcePacks:{}",
        serde_json::to_string(packs).unwrap_or_else(|_| "[]".into())
    );
    replace_or_append(options, |l| l.starts_with("resourcePacks:"), &value)
}

fn replace_or_append(text: &str, is_key: impl Fn(&str) -> bool, line: &str) -> String {
    let nl = eol(text);
    let mut out = Vec::new();
    let mut done = false;
    for l in lines(text) {
        if is_key(l) {
            if !done {
                out.push(line.to_owned());
                done = true;
            }
        } else {
            out.push(l.to_owned());
        }
    }
    if !done {
        out.push(line.to_owned());
    }
    let mut s = out.join(nl);
    s.push_str(nl);
    s
}

/// The new `resourcePacks` list after turning `id` on or off. Turning on
/// puts it on top (last); the vanilla pack is kept first on new lists.
pub fn toggle_pack(
    current: Option<Vec<String>>,
    id: &str,
    on: bool,
    file_prefix: bool,
) -> Vec<String> {
    let mut packs = current.unwrap_or_else(|| {
        if file_prefix {
            vec!["vanilla".to_owned()]
        } else {
            Vec::new()
        }
    });
    packs.retain(|p| p != id);
    if on {
        packs.push(id.to_owned());
    }
    packs
}

// ------------------------------------------------------------- properties --

/// Escape a Java properties value (keys here are fixed ASCII).
fn escape_value(v: &str) -> String {
    let mut s = String::with_capacity(v.len());
    for (i, c) in v.chars().enumerate() {
        match c {
            '\\' => s.push_str("\\\\"),
            ' ' if i == 0 => s.push_str("\\ "),
            '\t' => s.push_str("\\t"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    s.push_str(&format!("\\u{u:04X}"));
                }
            }
            c => s.push(c),
        }
    }
    s
}

fn unescape_value(v: &str) -> String {
    let mut units: Vec<u16> = Vec::new();
    let mut out = String::new();
    let flush = |units: &mut Vec<u16>, out: &mut String| {
        out.push_str(&String::from_utf16_lossy(units));
        units.clear();
    };
    let mut it = v.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            flush(&mut units, &mut out);
            out.push(c);
            continue;
        }
        match it.next() {
            Some('u') => {
                let hex: String = it.by_ref().take(4).collect();
                match u16::from_str_radix(&hex, 16) {
                    Ok(u) => units.push(u),
                    Err(_) => {
                        flush(&mut units, &mut out);
                        out.push_str(&hex);
                    }
                }
            }
            Some(e) => {
                flush(&mut units, &mut out);
                out.push(match e {
                    't' => '\t',
                    'n' => '\n',
                    'r' => '\r',
                    'f' => '\x0c',
                    other => other,
                });
            }
            None => {}
        }
    }
    flush(&mut units, &mut out);
    out
}

/// `key=value` / `key:value` / `key value` → (key, raw value). Comments and
/// blank lines → None. (Continuation lines aren't used by Iris.)
fn prop(line: &str) -> Option<(&str, &str)> {
    let l = line.trim_start();
    if l.is_empty() || l.starts_with('#') || l.starts_with('!') {
        return None;
    }
    let end = l.find(['=', ':', ' ', '\t']).unwrap_or(l.len());
    let key = &l[..end];
    let rest = l[end..].trim_start_matches([' ', '\t']);
    let rest = rest.strip_prefix(['=', ':']).unwrap_or(rest);
    Some((key, rest.trim_start_matches([' ', '\t'])))
}

fn get_prop(text: &str, key: &str) -> Option<String> {
    lines(text)
        .filter_map(prop)
        .filter(|(k, _)| *k == key)
        .last()
        .map(|(_, v)| unescape_value(v))
}

fn set_prop(text: &str, key: &str, value: &str) -> String {
    let line = format!("{key}={}", escape_value(value));
    replace_or_append(text, |l| prop(l).is_some_and(|(k, _)| k == key), &line)
}

/// The shader pack Iris uses, when shaders are on.
pub fn iris_shader(props: &str) -> Option<String> {
    let on = get_prop(props, "enableShaders").is_none_or(|v| v.trim() == "true");
    get_prop(props, "shaderPack").filter(|p| on && !p.is_empty())
}

/// Turn a shader pack on (`Some(file)`) or shaders off (`None`).
pub fn set_iris_shader(props: &str, pack: Option<&str>) -> String {
    match pack {
        Some(p) => set_prop(&set_prop(props, "shaderPack", p), "enableShaders", "true"),
        None => set_prop(props, "enableShaders", "false"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_by_version() {
        for v in ["1.13", "1.13.2", "1.21.1", "26.2", "24w14a", "1.21-pre1"] {
            assert!(uses_file_prefix(v), "{v}");
        }
        for v in ["1.12.2", "1.8.9", "17w43a", "b1.7.3"] {
            assert!(!uses_file_prefix(v), "{v}");
        }
        assert_eq!(pack_id("A b.zip", true), "file/A b.zip");
        assert_eq!(pack_id("A b.zip", false), "A b.zip");
    }

    #[test]
    fn resource_packs_line_is_replaced_and_everything_else_kept() {
        let opts = "version:3955\r\nfov:0.5\r\nresourcePacks:[\"vanilla\",\"fabric\"]\r\nincompatibleResourcePacks:[]\r\nlang:en_us\r\n";
        let cur = resource_packs(opts);
        assert_eq!(
            cur.as_deref(),
            Some(&["vanilla".to_owned(), "fabric".to_owned()][..])
        );
        let next = toggle_pack(cur, "file/x \"y\".zip", true, true);
        let out = set_resource_packs(opts, &next);
        assert_eq!(
            out,
            "version:3955\r\nfov:0.5\r\nresourcePacks:[\"vanilla\",\"fabric\",\"file/x \\\"y\\\".zip\"]\r\nincompatibleResourcePacks:[]\r\nlang:en_us\r\n"
        );
        assert_eq!(
            resource_packs(&out).unwrap().last().unwrap(),
            "file/x \"y\".zip"
        );
        // Turning on again moves it to the top; off removes it.
        let off = toggle_pack(resource_packs(&out), "file/x \"y\".zip", false, true);
        assert_eq!(off, ["vanilla", "fabric"]);
        // No options.txt yet / no line yet.
        assert_eq!(
            set_resource_packs("", &toggle_pack(None, "file/a.zip", true, true)),
            "resourcePacks:[\"vanilla\",\"file/a.zip\"]\n"
        );
        assert_eq!(toggle_pack(None, "a.zip", true, false), ["a.zip"]);
        assert_eq!(
            set_resource_packs("lang:en_us\n", &[]),
            "lang:en_us\nresourcePacks:[]\n"
        );
        // A broken line reads as "no list".
        assert!(resource_packs("resourcePacks:[oops").is_none());
    }

    #[test]
    fn iris_properties_round_trip() {
        let props = "#This file stores configuration options for Iris\n#Mon Jan 01\nmaxShadowRenderDistance=32\nshaderPack=Old.zip\nenableShaders=false\n";
        assert_eq!(iris_shader(props), None, "shaders off");
        let on = set_iris_shader(props, Some("Complementary Reimagined ü.zip"));
        assert!(
            on.contains("shaderPack=Complementary Reimagined \\u00FC.zip\n"),
            "{on}"
        );
        assert!(on.contains("enableShaders=true\n") && on.contains("maxShadowRenderDistance=32\n"));
        assert!(on.starts_with("#This file stores"));
        assert_eq!(
            iris_shader(&on).as_deref(),
            Some("Complementary Reimagined ü.zip")
        );
        let off = set_iris_shader(&on, None);
        assert_eq!(iris_shader(&off), None);
        assert!(
            off.contains("shaderPack=Complementary"),
            "the choice is remembered"
        );
        // A new file; a leading space and a backslash are escaped.
        let fresh = set_iris_shader("", Some(" a\\b.zip"));
        assert_eq!(fresh, "shaderPack=\\ a\\\\b.zip\nenableShaders=true\n");
        assert_eq!(iris_shader(&fresh).as_deref(), Some(" a\\b.zip"));
        // Missing enableShaders counts as on (Iris's default).
        assert_eq!(iris_shader("shaderPack = x.zip").as_deref(), Some("x.zip"));
        // Emoji go through surrogate pairs.
        let e = set_iris_shader("", Some("🌙.zip"));
        assert_eq!(iris_shader(&e).as_deref(), Some("🌙.zip"));
    }
}
