use serde_json::json;
use symfolinker_lib::constraint::{local_version, status, MISMATCH, SATISFIED, UNKNOWN};

/// The case this check exists for: a checkout from another major line, in an
/// application that pins the one it was written against.
#[test]
fn proves_a_mismatch_only_when_the_lines_cannot_overlap() {
    assert_eq!(status("^2.0", "3.1.0"), MISMATCH);
    assert_eq!(status("^2.0", "1.9.9"), MISMATCH);
    assert_eq!(status("~1.2", "2.0.0"), MISMATCH);
    assert_eq!(status(">=2.0", "1.0.0"), MISMATCH);
    assert_eq!(status("<2.0", "2.0.0"), MISMATCH);
    assert_eq!(status("1.*", "2.0.0"), MISMATCH);
    assert_eq!(status("^0.3", "0.4.0"), MISMATCH);
}

#[test]
fn confirms_the_versions_that_clearly_fit() {
    assert_eq!(status("^2.0", "2.0.0"), SATISFIED);
    assert_eq!(status("^2.0", "2.7.3"), SATISFIED);
    assert_eq!(status("^2.0", "v2.7.3"), SATISFIED);
    assert_eq!(status("~1.2", "1.9.0"), SATISFIED);
    assert_eq!(status("~1.2.3", "1.2.9"), SATISFIED);
    assert_eq!(status(">=1.0 <2.0", "1.5.0"), SATISFIED);
    assert_eq!(status(">= 1.0 <2.0", "1.5.0"), SATISFIED);
    assert_eq!(status("1.0 || 2.0", "2.0.1"), SATISFIED);
    assert_eq!(status("*", "7.0.0"), SATISFIED);
    assert_eq!(status("^0.3", "0.3.9"), SATISFIED);
    assert_eq!(status("^0.0.3", "0.0.3"), SATISFIED);
}

/// A checkout on a development branch is the normal case, and nothing about it can be
/// proven: answering `mismatch` there would make the check cry wolf on every link.
#[test]
fn says_unknown_rather_than_guessing() {
    assert_eq!(status("^2.0", "dev-main"), UNKNOWN);
    assert_eq!(status("dev-main", "2.0.0"), UNKNOWN);
    assert_eq!(status("@dev", "2.0.0"), UNKNOWN);
    assert_eq!(status("1.0 - 2.0", "1.5.0"), UNKNOWN);
    assert_eq!(status("!=1.0", "2.0.0"), UNKNOWN);
    assert_eq!(status("", "2.0.0"), UNKNOWN);
    assert_eq!(status("^2.0", ""), UNKNOWN);
    assert_eq!(status("^2.0", "not-a-version"), UNKNOWN);
    // Composer's fourth digit is not modelled, so it is not judged either.
    assert_eq!(status("^2.0", "2.0.0.1"), UNKNOWN);
    // An unreadable alternative may be the one that allows this version.
    assert_eq!(status("^9.0 || dev-main", "2.0.0"), UNKNOWN);
}

/// A version known to one part is a whole line, so it can sit partly inside a
/// constraint. That is neither a fit nor a conflict, and must not be reported as one.
#[test]
fn treats_an_imprecise_version_as_the_range_it_is() {
    assert_eq!(status("^2.0", "2.x-dev"), SATISFIED);
    assert_eq!(status("^3.0", "2.x-dev"), MISMATCH);
    assert_eq!(status("~2.3", "2.x-dev"), UNKNOWN);
    assert_eq!(status("^2.4", "2.4"), SATISFIED);
    assert_eq!(status("2.4.*", "2.4"), SATISFIED);
}

#[test]
fn contradicting_terms_can_never_be_satisfied() {
    assert_eq!(status(">=3.0 <2.0", "2.5.0"), MISMATCH);
}

#[test]
fn a_branch_alias_outranks_a_declared_version() {
    let extra = json!({"branch-alias": {"dev-main": "2.x-dev"}});
    assert_eq!(
        local_version(Some("1.0.0"), Some(&extra), Some("main")).as_deref(),
        Some("2.x-dev")
    );
    // An alias for another branch says nothing about the branch that is checked out.
    assert_eq!(
        local_version(Some("1.0.0"), Some(&extra), Some("feature/x")).as_deref(),
        Some("1.0.0")
    );
    assert_eq!(local_version(None, Some(&extra), Some("feature/x")), None);
    assert_eq!(local_version(None, None, None), None);
    assert_eq!(
        local_version(Some("3.2.1"), None, Some("main")).as_deref(),
        Some("3.2.1")
    );
}
