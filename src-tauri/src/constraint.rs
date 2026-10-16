//! Does the local checkout provide the version line the project asks for?
//!
//! A symlink bypasses Composer's resolver completely, so this never blocks a switch.
//! It answers the question linking raises anyway: the application requires `^2.0`, and
//! the checkout about to take that package's place is the 3.x branch. Composer would
//! have refused that; a symlink says nothing and the failure arrives later, as missing
//! classes or changed behaviour.
//!
//! Every answer is therefore either proven or `unknown`. A false alarm on a working
//! setup costs more trust than this warning can earn, so anything that cannot be
//! judged with certainty - `dev-main`, stability flags, hyphen ranges, four-part
//! versions - is reported as unknown rather than guessed at.

use serde_json::Value;

pub const SATISFIED: &str = "satisfied";
pub const MISMATCH: &str = "mismatch";
pub const UNKNOWN: &str = "unknown";

/// Versions compare as three parts; a missing part is a range, not a zero.
type Parts = [u64; 3];

const LOWEST: Parts = [0, 0, 0];
const HIGHEST: Parts = [u64::MAX, u64::MAX, u64::MAX];

/// A half-open version range `[lo, hi)`.
///
/// Both constraints and versions become ranges, which is what makes an honest
/// three-valued answer possible: `2.x-dev` is every 2.y.z, so against `~2.3` it
/// neither clearly fits nor clearly conflicts.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Range {
    lo: Parts,
    hi: Parts,
}

/// One alternative of a constraint, after its AND terms are combined.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Alternative {
    Allows(Range),
    /// The terms contradict each other, so no version can ever satisfy them.
    Never,
    /// A term this module does not understand; it may allow anything.
    Unknown,
}

/// The version line a local checkout declares, or `None` when it declares none.
///
/// A branch alias outranks an explicit version because that is exactly what Composer
/// itself uses to give a development branch a version line: a checkout on `main`
/// aliased to `2.x-dev` really does provide 2.x, whatever `version` says.
pub fn local_version(
    version: Option<&str>,
    extra: Option<&Value>,
    branch: Option<&str>,
) -> Option<String> {
    let alias = branch.zip(extra).and_then(|(branch, extra)| {
        extra
            .get("branch-alias")?
            .get(format!("dev-{branch}"))?
            .as_str()
            .map(str::to_owned)
    });
    alias.or_else(|| version.map(str::to_owned))
}

/// Compares a Composer constraint with the version line a checkout provides.
pub fn status(constraint: &str, version: &str) -> &'static str {
    let Some(wanted) = version_range(version) else {
        return UNKNOWN;
    };
    let mut conflicts = false;
    for alternative in constraint.split("||") {
        match combine(alternative) {
            // An alternative that cannot be read may well be the one that allows this
            // version, so the whole constraint stays unknown.
            Alternative::Unknown => return UNKNOWN,
            Alternative::Never => conflicts = true,
            Alternative::Allows(allowed) => {
                if allowed.lo <= wanted.lo && wanted.hi <= allowed.hi {
                    return SATISFIED;
                }
                if wanted.hi <= allowed.lo || allowed.hi <= wanted.lo {
                    conflicts = true;
                } else {
                    // Overlapping, so the checkout may or may not be inside it.
                    return UNKNOWN;
                }
            }
        }
    }
    if conflicts {
        MISMATCH
    } else {
        UNKNOWN
    }
}

/// The range a declared version covers: `2` is the whole 2 line, `2.3.1` one point.
fn version_range(version: &str) -> Option<Range> {
    let parts = numbers(version)?;
    Some(Range {
        lo: pad(&parts),
        hi: bump(&parts, parts.len() - 1),
    })
}

/// Intersects the AND terms of one alternative.
fn combine(alternative: &str) -> Alternative {
    let terms = terms(alternative);
    if terms.is_empty() {
        return Alternative::Unknown;
    }
    let mut range = Range {
        lo: LOWEST,
        hi: HIGHEST,
    };
    for term in terms {
        let Some(next) = term_range(&term) else {
            return Alternative::Unknown;
        };
        range = Range {
            lo: range.lo.max(next.lo),
            hi: range.hi.min(next.hi),
        };
        if range.lo >= range.hi {
            return Alternative::Never;
        }
    }
    Alternative::Allows(range)
}

/// Splits one alternative into AND terms, keeping an operator attached to its version
/// even when a space separates the two: `>= 1.0 <2.0` is two terms, not three.
fn terms(alternative: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for token in alternative
        .split([' ', ',', '\t'])
        .filter(|token| !token.is_empty())
    {
        match terms.last_mut() {
            Some(last) if last.chars().all(|part| "<>=~^!".contains(part)) => last.push_str(token),
            _ => terms.push(token.to_owned()),
        }
    }
    terms
}

/// The range one term allows, or `None` when the term is not understood.
fn term_range(term: &str) -> Option<Range> {
    let term = term.trim();
    if term == "*" {
        return Some(Range {
            lo: LOWEST,
            hi: HIGHEST,
        });
    }
    for (prefix, build) in [
        (">=", at_least as fn(&[u64]) -> Range),
        ("<=", at_most),
        (">", above),
        ("<", below),
        ("^", caret),
        ("~", tilde),
    ] {
        if let Some(rest) = term.strip_prefix(prefix) {
            return numbers(rest).map(|parts| build(&parts));
        }
    }
    let exact = term
        .strip_prefix("==")
        .or_else(|| term.strip_prefix('='))
        .unwrap_or(term);
    let parts = numbers(exact)?;
    // `2.3` pins the 2.3 line, not 2.3.0: Composer treats the unspecified part as free.
    Some(Range {
        lo: pad(&parts),
        hi: bump(&parts, parts.len() - 1),
    })
}

fn at_least(parts: &[u64]) -> Range {
    Range {
        lo: pad(parts),
        hi: HIGHEST,
    }
}

fn at_most(parts: &[u64]) -> Range {
    Range {
        lo: LOWEST,
        hi: bump(parts, parts.len() - 1),
    }
}

fn above(parts: &[u64]) -> Range {
    Range {
        lo: bump(parts, parts.len() - 1),
        hi: HIGHEST,
    }
}

fn below(parts: &[u64]) -> Range {
    Range {
        lo: LOWEST,
        hi: pad(parts),
    }
}

/// `^1.2` allows every 1.y; below 1.0 the first non-zero part is the stable one, so
/// `^0.3` allows every 0.3.z and `^0.0.3` only 0.0.3.
fn caret(parts: &[u64]) -> Range {
    let hi = match parts.iter().position(|part| *part != 0) {
        Some(index) => bump(parts, index),
        None => bump(parts, parts.len() - 1),
    };
    Range { lo: pad(parts), hi }
}

/// `~1.2.3` lets the last part rise, `~1.2` and `~1` let the minor rise.
fn tilde(parts: &[u64]) -> Range {
    Range {
        lo: pad(parts),
        hi: bump(parts, parts.len().saturating_sub(2).min(1)),
    }
}

/// The numeric parts of a version, or `None` when it is not a plain numeric version.
///
/// A wildcard ends the precise prefix, so `2.*` and `2.x-dev` both describe the 2
/// line. Pre-release and metadata suffixes are dropped: they cannot move a version
/// into another major line, which is what this comparison is about.
fn numbers(version: &str) -> Option<Vec<u64>> {
    let version = version.trim();
    let version = version.strip_prefix('v').unwrap_or(version);
    let version = version.split(['-', '+', '@']).next()?;
    let mut parts = Vec::new();
    for part in version.split('.') {
        match part.parse::<u64>() {
            Ok(number) => parts.push(number),
            Err(_) if matches!(part, "x" | "X" | "*") => break,
            Err(_) => return None,
        }
    }
    // Four parts is Composer's own fourth-digit convention, which this comparison does
    // not model; refusing it keeps the answer honest instead of silently truncating.
    (!parts.is_empty() && parts.len() <= 3).then_some(parts)
}

fn pad(parts: &[u64]) -> Parts {
    let mut padded = LOWEST;
    for (slot, part) in padded.iter_mut().zip(parts) {
        *slot = *part;
    }
    padded
}

/// The first version above `parts` when the part at `index` is incremented.
fn bump(parts: &[u64], index: usize) -> Parts {
    let mut bumped = pad(parts);
    bumped[index] = bumped[index].saturating_add(1);
    for slot in bumped.iter_mut().skip(index + 1) {
        *slot = 0;
    }
    bumped
}
