//! The protocol version table and the feature-introduction checks (A6).
//!
//! [`SUPPORTED_VERSIONS`] is the one ordered list of every `Program.version`
//! this contract accepts, newest first. A feature gate names the version
//! that introduced the feature and asks [`version_at_least`]. Publishing a
//! new version therefore changes this table and [`crate::VERSION`] only: every
//! gate keeps its meaning for every older version, and the new version gets
//! every feature introduced at or before it. A gate that must match one exact
//! version (for example a stored program pinned for its digest) compares
//! that version explicitly and says why.
//!
//! See `docs/contract/core-v1.md` section 11.
use crate::{LEGACY_VERSION, VERSION};

/// Every `Program.version` this contract decodes, newest first.
///
/// Each version keeps the exact semantics it was published with. Features
/// newer than a program's declared version are rejected at execution with
/// `E_VERSION`. 1.0.0 freezes core v1 as published in 0.24.0: it has the
/// same features as 0.24.0. 1.1.0 (B1) adds `reader_groups` on fact records
/// and `on_behalf_of` in engine-recorded provenance. 1.2.0 (B2) adds
/// `widen_readers` on corrections and the write-time rules of ADR-0008
/// section 3 (declared derivations and readers widened by a correction).
/// 1.3.0 (B5) adds `review` in engine-recorded provenance: the proposal,
/// decision and reviewers of a fact published through review.
pub const SUPPORTED_VERSIONS: &[&str] = &[
    VERSION,
    "1.2.0",
    "1.1.0",
    "1.0.0",
    "0.24.0",
    "0.23.0",
    "0.22.0",
    "0.21.0",
    "0.20.0",
    "0.19.0",
    "0.18.0",
    "0.17.0",
    "0.16.0",
    "0.15.0",
    "0.14.0",
    "0.13.0",
    "0.12.0",
    "0.11.0",
    "0.10.0",
    "0.9.0",
    "0.8.0",
    "0.7.0",
    "0.6.0",
    "0.5.0",
    "0.4.0",
    "0.3.0",
    "0.2.0",
    LEGACY_VERSION,
];

/// True when `version` is a contract version this crate decodes.
pub fn is_supported_version(version: &str) -> bool {
    SUPPORTED_VERSIONS.contains(&version)
}

/// Position of `version` in publication order (0 is the oldest), or `None`
/// for a version this contract does not know.
pub fn version_rank(version: &str) -> Option<usize> {
    SUPPORTED_VERSIONS
        .iter()
        .position(|v| *v == version)
        .map(|i| SUPPORTED_VERSIONS.len() - 1 - i)
}

/// True when `version` is a known version published at or after
/// `introduced`, the version that introduced a feature. An unknown
/// `version` has no features.
///
/// # Panics
///
/// When `introduced` is not in [`SUPPORTED_VERSIONS`]: gates name published
/// versions only, so this is a programming error, never an input error.
pub fn version_at_least(version: &str, introduced: &str) -> bool {
    let floor = version_rank(introduced)
        .unwrap_or_else(|| panic!("feature gate names an unknown version {introduced}"));
    version_rank(version).is_some_and(|rank| rank >= floor)
}

/// True when `version` is the current protocol, [`crate::VERSION`].
pub fn is_current(version: &str) -> bool {
    version == VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_ordered_unique_and_starts_at_the_current_version() {
        assert_eq!(SUPPORTED_VERSIONS[0], VERSION);
        assert_eq!(*SUPPORTED_VERSIONS.last().unwrap(), LEGACY_VERSION);
        let parsed: Vec<(u64, u64, u64)> = SUPPORTED_VERSIONS
            .iter()
            .map(|v| {
                let parts: Vec<u64> = v.split('.').map(|p| p.parse().unwrap()).collect();
                assert_eq!(parts.len(), 3, "{v}");
                (parts[0], parts[1], parts[2])
            })
            .collect();
        for pair in parsed.windows(2) {
            assert!(pair[0] > pair[1], "strictly newest first: {pair:?}");
        }
    }

    #[test]
    fn version_at_least_follows_publication_order() {
        for (i, v) in SUPPORTED_VERSIONS.iter().enumerate() {
            for (j, introduced) in SUPPORTED_VERSIONS.iter().enumerate() {
                assert_eq!(
                    version_at_least(v, introduced),
                    i <= j,
                    "{v} >= {introduced}"
                );
            }
        }
        assert!(
            !version_at_least("1.4.0", "0.1.0"),
            "unknown versions have no features"
        );
        assert!(!version_at_least("", "0.1.0"));
        assert!(version_at_least(VERSION, "0.24.0"));
        assert!(version_at_least(VERSION, "1.0.0"));
        assert!(!version_at_least("1.0.0", VERSION));
        assert!(is_current(VERSION) && !is_current("1.0.0"));
    }

    #[test]
    #[should_panic(expected = "unknown version")]
    fn a_gate_on_an_unpublished_version_is_a_programming_error() {
        version_at_least(VERSION, "1.4.0");
    }
}
