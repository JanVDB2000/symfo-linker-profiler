//! Will the linked checkout be found by the autoloader that is already generated?
//!
//! Composer generates `vendor/composer/autoload_psr4.php` from the autoload rules of
//! the versions it installed. A symlink replaces the directory those rules point at,
//! but not the rules themselves: a checkout that moved its sources, renamed a
//! namespace or added a `files` entry is mounted correctly and still produces "class
//! not found", with every status in this app green.
//!
//! `vendor/composer/installed.json` is what the generated autoloader was built from,
//! so it is the right side of the comparison - not the package directory, which the
//! link has already replaced.
//!
//! Paths are normalised before comparing, because `src/` and `src` mean the same
//! thing to Composer, and a warning nobody can act on is worse than none.

use serde_json::{Map, Value};

pub const MATCHES: &str = "matches";
pub const DIFFERS: &str = "differs";
pub const UNKNOWN: &str = "unknown";

/// Compares the autoload rules Composer installed with the ones the checkout declares.
///
/// Either side missing is `unknown`: a package installed from a path repository has no
/// entry in `installed.json`, and a checkout without autoload rules of its own cannot
/// be judged against the ones in force.
pub fn status(installed: Option<&Value>, local: Option<&Value>) -> &'static str {
    match (installed, local) {
        (Some(installed), Some(local)) if normalize(installed) == normalize(local) => MATCHES,
        (Some(_), Some(_)) => DIFFERS,
        _ => UNKNOWN,
    }
}

/// The autoload rules of one package, as recorded in `vendor/composer/installed.json`.
///
/// Composer 2 wraps the list in an object; Composer 1 wrote a bare array, and both are
/// read because the file is written by whichever version the project uses.
pub fn installed_autoload(installed_json: &Value, package: &str) -> Option<Value> {
    let packages = installed_json
        .get("packages")
        .unwrap_or(installed_json)
        .as_array()?;
    packages
        .iter()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(package))
        .and_then(|entry| entry.get("autoload"))
        .cloned()
}

/// Canonical form of an autoload section: comparable paths, order-independent lists.
///
/// Sorting the lists is deliberate. Reordering a classmap changes which file wins a
/// duplicate class name, but that is a Composer-level subtlety, not the switch-level
/// breakage this check is for, and flagging it would make the common case noisy.
///
/// The recursion below needs no depth limit of its own: `serde_json` refuses to
/// deserialize beyond its own nesting limit, so a value that exists here is already
/// shallow enough to walk.
fn normalize(autoload: &Value) -> Value {
    let Some(sections) = autoload.as_object() else {
        return Value::Null;
    };
    Value::Object(
        sections
            .iter()
            .map(|(section, value)| (section.clone(), normalize_section(value)))
            .filter(|(_, value)| !is_empty(value))
            .collect::<Map<String, Value>>(),
    )
}

/// `psr-4` and `psr-0` map a namespace to one or more paths; the other sections are
/// plain path lists. Both shapes end up as a sorted list of normalised paths.
fn normalize_section(value: &Value) -> Value {
    match value {
        Value::Object(namespaces) => Value::Object(
            namespaces
                .iter()
                .map(|(namespace, paths)| (namespace.clone(), normalize_section(paths)))
                .collect(),
        ),
        Value::Array(paths) => {
            let mut normalized: Vec<Value> = paths.iter().map(normalize_path).collect();
            normalized.sort_by_key(|path| path.to_string());
            Value::Array(normalized)
        }
        other => Value::Array(vec![normalize_path(other)]),
    }
}

/// `./src/`, `src/` and `src` are one directory; an empty string and `.` are the root.
fn normalize_path(path: &Value) -> Value {
    let Some(text) = path.as_str() else {
        return path.clone();
    };
    let trimmed = text.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_prefix("./").unwrap_or(trimmed);
    Value::String(if trimmed == "." {
        String::new()
    } else {
        trimmed.to_owned()
    })
}

/// An empty section carries no rules, so its presence must not make two equal
/// autoload configurations compare as different.
fn is_empty(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.is_empty(),
        Value::Object(entries) => entries.is_empty(),
        Value::Null => true,
        _ => false,
    }
}
