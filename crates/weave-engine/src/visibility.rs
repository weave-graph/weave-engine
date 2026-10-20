//! The one record-visibility predicate and the one reader intersection
//! (ADR-0007 sections 2 and 13). No other code re-implements the reader rule.
//!
//! A request runs under an [`Access`]: an AND of reader [`Clause`]s resolved
//! once from the principal registry (or, in a store without a registry, the
//! single literal principal of the trusted host). A user has one clause. An
//! agent has one clause per link of its ownership chain plus its own, so it
//! never sees a record its owner cannot see (ADR-0007 section 3).
use std::collections::BTreeSet;
use std::sync::Arc;

/// One OR clause of a request's identity: a record matches it when the
/// record's `readers` name one of `principals` or its `reader_groups` name
/// one of `groups`.
///
/// `principals` never holds a group, so a group ID listed in `readers` is a
/// literal that matches no one (ADR-0007 section 1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Clause {
    pub(crate) principals: BTreeSet<String>,
    pub(crate) groups: BTreeSet<String>,
}

impl Clause {
    fn admits(&self, readers: &[String], reader_groups: &[String]) -> bool {
        readers.iter().any(|r| self.principals.contains(r))
            || reader_groups.iter().any(|g| self.groups.contains(g))
    }
}

/// The resolved identity of one request: every clause must match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Access {
    clauses: Vec<Clause>,
}

impl Access {
    /// The local trusted-host profile: exactly today's literal-principal rule.
    pub(crate) fn literal(principal: &str) -> Arc<Self> {
        Arc::new(Self {
            clauses: vec![Clause {
                principals: BTreeSet::from([principal.to_owned()]),
                groups: BTreeSet::new(),
            }],
        })
    }
    /// Access that matches no restricted record (an unregistered, suspended or
    /// group principal in a store with a registry). Public records still match.
    pub(crate) fn nobody() -> Arc<Self> {
        Arc::new(Self {
            clauses: vec![Clause::default()],
        })
    }
    pub(crate) fn from_clauses(clauses: Vec<Clause>) -> Arc<Self> {
        if clauses.is_empty() {
            return Self::nobody();
        }
        Arc::new(Self { clauses })
    }
    /// True when every clause can use `group` (ADR-0007 section 2: a write may
    /// name only groups the writer can discover).
    pub(crate) fn has_group(&self, group: &str) -> bool {
        self.clauses.iter().all(|c| c.groups.contains(group))
    }
}

/// True when a record with these `readers` and `reader_groups` is visible
/// under `access`. Both empty means public. Otherwise every clause of the
/// access must match a listed principal or a listed group.
///
/// For a record without groups under [`Access::literal`] this is exactly the
/// pre-B1 rule: empty `readers` is public, otherwise the principal is listed.
pub(crate) fn record_admits(readers: &[String], reader_groups: &[String], access: &Access) -> bool {
    (readers.is_empty() && reader_groups.is_empty())
        || access
            .clauses
            .iter()
            .all(|c| c.admits(readers, reader_groups))
}

/// The literal-reader rule for policy objects that have no `reader_groups`
/// (governance and identity policies): empty is public.
pub(crate) fn readers_admit(readers: &[String], access: &Access) -> bool {
    record_admits(readers, &[], access)
}

/// One input restriction of a derived record.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Restriction<'a> {
    pub(crate) readers: &'a [String],
    pub(crate) reader_groups: &'a [String],
}

impl Restriction<'_> {
    fn public(&self) -> bool {
        self.readers.is_empty() && self.reader_groups.is_empty()
    }
}

/// The restriction of a record derived from several records: an AND of the
/// inputs' reader clauses (ADR-0007 section 2). Group-based clauses are never
/// resolved into literal readers, so revocation keeps working.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Derived {
    /// No input names a group: the literal intersection, exactly as the
    /// engine has always computed it.
    Literal(Vec<String>),
    /// Some input names a group: each restricted input's clause, all of
    /// which a reader must match. B2 (ADR-0008) owns its write-time form.
    Conjunction(Vec<(Vec<String>, Vec<String>)>),
}

impl Derived {
    /// The single `(readers, reader_groups)` pair a record field can hold.
    ///
    /// A literal intersection and a conjunction of at most one restricted
    /// clause are exact. For two or more clauses the pair is their
    /// clause-wise meet (common principals and common groups). The meet
    /// admits only readers that every clause admits, so it never widens a
    /// restriction and never resolves membership. It can be narrower than the
    /// exact conjunction; B2 replaces it with the carrier form of ADR-0008.
    pub(crate) fn encode(&self) -> (Vec<String>, Vec<String>) {
        match self {
            Derived::Literal(readers) => (readers.clone(), Vec::new()),
            Derived::Conjunction(clauses) => {
                let mut clauses = clauses.iter();
                let Some((first_readers, first_groups)) = clauses.next() else {
                    return (Vec::new(), Vec::new());
                };
                let (mut readers, mut groups) = (first_readers.clone(), first_groups.clone());
                for (r, g) in clauses {
                    readers.retain(|p| r.contains(p));
                    groups.retain(|p| g.contains(p));
                }
                if readers.is_empty() && groups.is_empty() {
                    // An empty meet must stay restricted, never become public.
                    return (vec![MATCHES_NO_ONE.to_owned()], Vec::new());
                }
                (readers, groups)
            }
        }
    }
}

/// A literal reader that no request principal can carry: principal IDs in the
/// `weave:` namespace are never issued to users or agents (ADR-0007).
pub(crate) const MATCHES_NO_ONE: &str = "weave:no-reader";

/// The restriction of a record derived from all of `inputs`.
pub(crate) fn intersect(inputs: &[Restriction<'_>]) -> Derived {
    if inputs.iter().all(|r| r.reader_groups.is_empty()) {
        let mut readers: Option<Vec<String>> = None;
        for input in inputs {
            readers = Some(match readers {
                None => input.readers.to_vec(),
                Some(current) => intersect_readers(&current, input.readers),
            });
        }
        return Derived::Literal(readers.unwrap_or_default());
    }
    Derived::Conjunction(
        inputs
            .iter()
            .filter(|r| !r.public())
            .map(|r| (r.readers.to_vec(), r.reader_groups.to_vec()))
            .collect(),
    )
}

/// Readers of a record derived from two records, as the explicit-assertion
/// materialization has always computed them: an empty side defers to the
/// other side, and two restricted sides keep their common principals.
fn intersect_readers(left: &[String], right: &[String]) -> Vec<String> {
    if left.is_empty() {
        right.to_vec()
    } else if right.is_empty() {
        left.to_vec()
    } else {
        left.iter().filter(|p| right.contains(p)).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    fn clause(p: &[&str], g: &[&str]) -> Clause {
        Clause {
            principals: p.iter().map(|x| x.to_string()).collect(),
            groups: g.iter().map(|x| x.to_string()).collect(),
        }
    }

    #[test]
    fn literal_access_is_the_pre_b1_rule() {
        let alice = Access::literal("alice");
        for readers in [s(&[]), s(&["alice"]), s(&["bob"]), s(&["bob", "alice"])] {
            let before = readers.is_empty() || readers.iter().any(|r| r == "alice");
            assert_eq!(record_admits(&readers, &[], &alice), before, "{readers:?}");
        }
        // A record with groups only is not public, and literal access has no groups.
        assert!(!record_admits(&[], &s(&["team"]), &alice));
    }

    #[test]
    fn groups_never_match_through_readers_and_clauses_conjoin() {
        let user = Access::from_clauses(vec![clause(&["u"], &["team"])]);
        assert!(record_admits(&[], &s(&["team"]), &user));
        assert!(
            !record_admits(&s(&["team"]), &[], &user),
            "a group in readers is a literal"
        );
        let agent = Access::from_clauses(vec![clause(&["u", "a"], &["team"]), clause(&["a"], &[])]);
        assert!(record_admits(&s(&["a"]), &[], &agent));
        assert!(
            !record_admits(&s(&["u"]), &[], &agent),
            "the agent's own clause"
        );
        assert!(
            !record_admits(&[], &s(&["team"]), &agent),
            "group not exercised"
        );
        assert!(record_admits(&[], &[], &agent));
        assert!(!agent.has_group("team") && user.has_group("team"));
        assert!(!record_admits(&s(&["x"]), &s(&["y"]), &Access::nobody()));
    }

    #[test]
    fn intersections_keep_literal_bytes_and_never_widen_groups() {
        let r = |readers: &'static [String], groups: &'static [String]| Restriction {
            readers,
            reader_groups: groups,
        };
        let a: &'static [String] = Box::leak(s(&["a", "b"]).into_boxed_slice());
        let b: &'static [String] = Box::leak(s(&["b", "c"]).into_boxed_slice());
        let none: &'static [String] = &[];
        let team: &'static [String] = Box::leak(s(&["team"]).into_boxed_slice());
        assert_eq!(
            intersect(&[r(a, none), r(b, none)]),
            Derived::Literal(s(&["b"]))
        );
        assert_eq!(
            intersect(&[r(none, none), r(b, none)]),
            Derived::Literal(s(&["b", "c"]))
        );
        let derived = intersect(&[r(a, none), r(none, team)]);
        assert_eq!(
            derived,
            Derived::Conjunction(vec![(s(&["a", "b"]), vec![]), (vec![], s(&["team"]))])
        );
        // The meet of disjoint clauses stays restricted.
        let (readers, groups) = derived.encode();
        assert!(!(readers.is_empty() && groups.is_empty()));
        for access in [
            Access::from_clauses(vec![clause(&["a"], &[])]),
            Access::from_clauses(vec![clause(&["z"], &["team"])]),
            Access::from_clauses(vec![clause(&["a"], &["team"])]),
        ] {
            assert!(
                !record_admits(&readers, &groups, &access) || {
                    record_admits(a, none, &access) && record_admits(none, team, &access)
                }
            );
        }
        // One restricted clause is exact.
        assert_eq!(
            intersect(&[r(none, none), r(b, team)]).encode(),
            (s(&["b", "c"]), s(&["team"]))
        );
    }
}
