//! Backend conformance suite for `fact_store::FactBackend` (ADR-0004 section 6).
//!
//! Every check runs against the in-memory reference backend and the SQLite
//! backend. A differential test also replays one operation sequence on both
//! and compares every read at every revision. SQLite-only tests cover
//! durability, which the reference backend does not claim.
use rusqlite::Connection;
use weave_engine::fact_store::{
    memory::MemoryFactStore,
    sqlite::{self, SqliteFactStore},
    BaseState, FactBackend, FactFilter, FactKind, FactOp, FactVersion, IndexKeys, Namespace,
    NewRevision, Recording, StoredFact, ValidTime, MAX_BODY_BYTES,
};

fn node(id: &str, entity: &str, valid: Option<(i64, Option<i64>)>) -> FactVersion {
    FactVersion {
        kind: FactKind::Node,
        id: id.into(),
        body: format!(r#"{{"id":"{id}","entity_id":"{entity}"}}"#),
        keys: IndexKeys {
            entity: Some(entity.into()),
            subject: Some(id.into()),
            valid: valid.map(|(start, end)| ValidTime { start, end }),
            ..IndexKeys::default()
        },
    }
}

fn edge(
    id: &str,
    from: &str,
    to: &str,
    predicate: &str,
    start: i64,
    end: Option<i64>,
) -> FactVersion {
    FactVersion {
        kind: FactKind::Edge,
        id: id.into(),
        body: format!(
            r#"{{"id":"{id}","from":"{from}","to":"{to}","predicate":"{predicate}","start":{start}}}"#
        ),
        keys: IndexKeys {
            subject: Some(from.into()),
            object: Some(to.into()),
            predicate: Some(predicate.into()),
            valid: Some(ValidTime { start, end }),
            ..IndexKeys::default()
        },
    }
}

fn attachment(id: &str, host: &str, key: &str) -> FactVersion {
    FactVersion {
        kind: FactKind::Attachment,
        id: id.into(),
        body: format!(r#"{{"id":"{id}","host":"{host}","key":"{key}"}}"#),
        keys: IndexKeys {
            subject: Some(host.into()),
            predicate: Some(key.into()),
            valid: Some(ValidTime {
                start: 0,
                end: None,
            }),
            ..IndexKeys::default()
        },
    }
}

fn rev(revision: &str, parent: Option<&str>, at: i64, ops: Vec<FactOp>) -> NewRevision {
    NewRevision {
        revision: revision.into(),
        graph_id: "g".into(),
        branch_id: "main".into(),
        parent: parent.map(Into::into),
        writer: "writer".into(),
        recorded_at_ms: at,
        ops,
    }
}

fn empty_base() -> Option<BaseState> {
    Some(BaseState {
        header: "{}".into(),
        facts: vec![],
    })
}

fn code(result: weave_engine::Result<impl std::fmt::Debug>) -> String {
    result.expect_err("expected an error").code
}

fn ids(facts: &[StoredFact]) -> Vec<String> {
    facts.iter().map(|f| f.version.id.clone()).collect()
}

fn all(b: &dyn FactBackend, revision: &str) -> Vec<String> {
    ids(&b.select(revision, &FactFilter::default(), 1000).unwrap())
}

/// r1: n1, n2, e1(n1→n2). r2: correct n1, retract e1, add e2.
fn two_revisions(b: &mut dyn FactBackend) {
    b.append(
        rev(
            "r1",
            None,
            10,
            vec![
                FactOp::Assert(node("n1", "E1", None)),
                FactOp::Assert(node("n2", "E2", Some((10, Some(50))))),
                FactOp::Assert(edge("e1", "n1", "n2", "p", 0, None)),
            ],
        ),
        empty_base(),
    )
    .unwrap();
    let mut corrected = node("n1", "E1", None);
    corrected.body = r#"{"id":"n1","entity_id":"E1","v":2}"#.into();
    b.append(
        rev(
            "r2",
            Some("r1"),
            20,
            vec![
                FactOp::Assert(corrected),
                FactOp::Retract {
                    kind: FactKind::Edge,
                    id: "e1".into(),
                },
                FactOp::Assert(edge("e2", "n2", "n1", "q", 5, Some(15))),
            ],
        ),
        None,
    )
    .unwrap();
}

fn append_and_read_history(b: &mut dyn FactBackend) {
    two_revisions(b);
    assert_eq!(all(b, "r1"), ["n1", "n2", "e1"]);
    assert_eq!(all(b, "r2"), ["n1", "n2", "e2"]);
    let old = b.get("r1", Namespace::Node, "n1").unwrap().unwrap();
    let new = b.get("r2", Namespace::Node, "n1").unwrap().unwrap();
    assert!(!old.version.body.contains("\"v\":2"));
    assert!(new.version.body.contains("\"v\":2"));
    assert_eq!(old.recording.revision, "r1");
    assert_eq!(new.recording.revision, "r2");
    assert!(b.get("r1", Namespace::Record, "e1").unwrap().is_some());
    assert!(b.get("r2", Namespace::Record, "e1").unwrap().is_none());
    // Unchanged facts keep their first recording.
    let n2 = b.get("r2", Namespace::Node, "n2").unwrap().unwrap();
    assert_eq!(
        (n2.recording.revision.as_str(), n2.recording.recorded_at_ms),
        ("r1", 10)
    );
    let info = b.revision("r2").unwrap().unwrap();
    assert_eq!(
        (info.seq, info.parent.as_deref(), info.op_count),
        (2, Some("r1"), 3)
    );
    assert!(b.is_tip("r2").unwrap() && !b.is_tip("r1").unwrap());
}

fn positions_are_contiguous_and_increasing(b: &mut dyn FactBackend) {
    two_revisions(b);
    let mut last = 0;
    for revision in ["r1", "r2"] {
        let info = b.revision(revision).unwrap().unwrap();
        let ops = b.ops(revision).unwrap();
        assert_eq!(ops.len() as i64, info.op_count);
        for (offset, op) in ops.iter().enumerate() {
            assert_eq!(op.position, info.first_position + offset as i64);
            assert!(op.position > last);
            last = op.position;
        }
    }
    let ops = b.ops("r2").unwrap();
    assert!(matches!(&ops[1].op, FactOp::Retract { kind: FactKind::Edge, id } if id == "e1"));
    assert!(
        matches!(&ops[2].op, FactOp::Assert(v) if v == &edge("e2", "n2", "n1", "q", 5, Some(15)))
    );
}

fn base_state_rules(b: &mut dyn FactBackend) {
    let op = || vec![FactOp::Assert(node("n1", "E1", None))];
    assert_eq!(
        code(b.append(rev("r1", None, 1, op()), None)),
        "E_INTEGRITY"
    );
    assert!(b.revision("r1").unwrap().is_none());
    b.append(rev("r1", None, 1, op()), empty_base()).unwrap();
    assert_eq!(
        code(b.append(
            rev(
                "r2",
                Some("r1"),
                2,
                vec![FactOp::Assert(node("n2", "E", None))]
            ),
            empty_base()
        )),
        "E_INTEGRITY"
    );
    assert!(b.revision("r2").unwrap().is_none());
    // A parent that is not a fact revision (for example a snapshot) starts a line.
    let base = BaseState {
        header: r#"{"profile":"legacy"}"#.into(),
        facts: vec![(
            node("s1", "S", None),
            Recording {
                writer: "snapshot-writer".into(),
                recorded_at_ms: 1,
                revision: "sha256:snapshot".into(),
            },
        )],
    };
    let info = b
        .append(
            rev(
                "r3",
                Some("sha256:snapshot"),
                3,
                vec![FactOp::Assert(node("n3", "E", None))],
            ),
            Some(base),
        )
        .unwrap();
    assert_eq!(info.seq, 1);
    assert_eq!(all(b, "r3"), ["n3", "s1"]);
    assert_eq!(b.header("r3").unwrap(), r#"{"profile":"legacy"}"#);
    assert_eq!(b.header("r1").unwrap(), "{}");
    let copied = b.get("r3", Namespace::Node, "s1").unwrap().unwrap();
    assert_eq!(copied.recording.writer, "snapshot-writer");
    assert_eq!(copied.recording.revision, "sha256:snapshot");
    // Base entries are not operations of the revision.
    assert_eq!(b.ops("r3").unwrap().len(), 1);
}

fn fork_starts_independent_line(b: &mut dyn FactBackend) {
    two_revisions(b);
    let base = BaseState {
        header: "{}".into(),
        facts: b
            .select("r1", &FactFilter::default(), 100)
            .unwrap()
            .into_iter()
            .map(|f| (f.version, f.recording))
            .collect(),
    };
    let fork = b
        .append(
            rev(
                "f1",
                Some("r1"),
                30,
                vec![FactOp::Retract {
                    kind: FactKind::Node,
                    id: "n2".into(),
                }],
            ),
            Some(base),
        )
        .unwrap();
    let main = b.revision("r2").unwrap().unwrap();
    assert_ne!(fork.line, main.line);
    assert!(b.is_tip("f1").unwrap() && b.is_tip("r2").unwrap());
    assert_eq!(all(b, "f1"), ["n1", "e1"]);
    assert_eq!(all(b, "r2"), ["n1", "n2", "e2"]);
    assert_eq!(all(b, "r1"), ["n1", "n2", "e1"]);
    b.append(
        rev(
            "r3",
            Some("r2"),
            31,
            vec![FactOp::Assert(node("n9", "E", None))],
        ),
        None,
    )
    .unwrap();
    assert_eq!(all(b, "f1"), ["n1", "e1"]);
}

fn revision_ids_are_never_reused(b: &mut dyn FactBackend) {
    two_revisions(b);
    assert_eq!(
        code(b.append(
            rev(
                "r2",
                Some("r2"),
                30,
                vec![FactOp::Assert(node("x", "E", None))]
            ),
            None
        )),
        "E_CONFLICT"
    );
    assert_eq!(
        code(b.append(
            rev("r1", None, 30, vec![FactOp::Assert(node("x", "E", None))]),
            empty_base()
        )),
        "E_CONFLICT"
    );
}

fn operation_rules_and_atomic_failure(b: &mut dyn FactBackend) {
    two_revisions(b);
    let before = all(b, "r2");
    let attempts: Vec<Vec<FactOp>> = vec![
        vec![],
        vec![
            FactOp::Assert(node("x", "E", None)),
            FactOp::Assert(node("x", "E", None)),
        ],
        vec![
            FactOp::Assert(node("y", "E", None)),
            FactOp::Retract {
                kind: FactKind::Edge,
                id: "e1".into(),
            },
        ],
        vec![
            FactOp::Assert(node("y", "E", None)),
            FactOp::Assert(attachment("e2", "n1", "k")),
        ],
        vec![FactOp::Retract {
            kind: FactKind::Attachment,
            id: "e2".into(),
        }],
    ];
    for ops in attempts {
        assert_eq!(
            code(b.append(rev("bad", Some("r2"), 30, ops), None)),
            "E_CONFLICT"
        );
        assert!(b.revision("bad").unwrap().is_none());
        assert!(b.is_tip("r2").unwrap());
        assert_eq!(all(b, "r2"), before);
    }
    // Node and record namespaces are distinct: a node and an edge may share an ID.
    b.append(
        rev(
            "r3",
            Some("r2"),
            30,
            vec![FactOp::Assert(edge("n1", "n1", "n2", "p", 0, None))],
        ),
        None,
    )
    .unwrap();
    assert_eq!(
        b.get("r3", Namespace::Node, "n1")
            .unwrap()
            .unwrap()
            .version
            .kind,
        FactKind::Node
    );
    assert_eq!(
        b.get("r3", Namespace::Record, "n1")
            .unwrap()
            .unwrap()
            .version
            .kind,
        FactKind::Edge
    );
}

fn recorded_time_is_verbatim_and_monotonic(b: &mut dyn FactBackend) {
    two_revisions(b);
    let fact = b.get("r2", Namespace::Record, "e2").unwrap().unwrap();
    assert_eq!(
        fact.recording,
        Recording {
            writer: "writer".into(),
            recorded_at_ms: 20,
            revision: "r2".into()
        }
    );
    let info = b.revision("r2").unwrap().unwrap();
    assert_eq!((info.writer.as_str(), info.recorded_at_ms), ("writer", 20));
    assert_eq!(
        code(b.append(
            rev(
                "r3",
                Some("r2"),
                19,
                vec![FactOp::Assert(node("x", "E", None))]
            ),
            None
        )),
        "E_HISTORY_CLOCK"
    );
    b.append(
        rev(
            "r3",
            Some("r2"),
            20,
            vec![FactOp::Assert(node("x", "E", None))],
        ),
        None,
    )
    .unwrap();
    let base = BaseState {
        header: "{}".into(),
        facts: vec![(
            node("late", "E", None),
            Recording {
                writer: "w".into(),
                recorded_at_ms: 99,
                revision: "r0".into(),
            },
        )],
    };
    assert_eq!(
        code(b.append(
            rev("f", None, 50, vec![FactOp::Assert(node("x", "E", None))]),
            Some(base)
        )),
        "E_HISTORY_CLOCK"
    );
}

fn select_by_index_keys(b: &mut dyn FactBackend) {
    two_revisions(b);
    b.append(
        rev(
            "r3",
            Some("r2"),
            40,
            vec![
                FactOp::Assert(attachment("a1", "n1", "firmware")),
                FactOp::Assert(node("n3", "E1", Some((100, None)))),
            ],
        ),
        None,
    )
    .unwrap();
    let select = |filter: FactFilter| ids(&b.select("r3", &filter, 100).unwrap());
    assert_eq!(
        select(FactFilter {
            entity: Some("E1".into()),
            ..Default::default()
        }),
        ["n1", "n3"]
    );
    assert_eq!(
        select(FactFilter {
            subject: Some("n1".into()),
            ..Default::default()
        }),
        ["n1", "a1"]
    );
    assert_eq!(
        select(FactFilter {
            object: Some("n1".into()),
            ..Default::default()
        }),
        ["e2"]
    );
    assert_eq!(
        select(FactFilter {
            predicate: Some("q".into()),
            ..Default::default()
        }),
        ["e2"]
    );
    assert_eq!(
        select(FactFilter {
            predicate: Some("firmware".into()),
            ..Default::default()
        }),
        ["a1"]
    );
    assert_eq!(
        select(FactFilter {
            kind: Some(FactKind::Node),
            ..Default::default()
        }),
        ["n1", "n2", "n3"]
    );
    // Half-open valid time; facts without valid time match every instant.
    assert_eq!(
        select(FactFilter {
            valid_at: Some(5),
            ..Default::default()
        }),
        ["n1", "e2", "a1"]
    );
    assert_eq!(
        select(FactFilter {
            valid_at: Some(15),
            ..Default::default()
        }),
        ["n1", "n2", "a1"]
    );
    assert_eq!(
        select(FactFilter {
            valid_at: Some(50),
            ..Default::default()
        }),
        ["n1", "a1"]
    );
    assert_eq!(
        select(FactFilter {
            valid_at: Some(100),
            ..Default::default()
        }),
        ["n1", "n3", "a1"]
    );
    // Recorded time of the live version, half-open.
    assert_eq!(
        select(FactFilter {
            recorded: Some((10, 20)),
            ..Default::default()
        }),
        ["n2"]
    );
    assert_eq!(
        select(FactFilter {
            recorded: Some((20, 41)),
            ..Default::default()
        }),
        ["n1", "n3", "e2", "a1"]
    );
    assert_eq!(
        select(FactFilter {
            kind: Some(FactKind::Node),
            entity: Some("E1".into()),
            valid_at: Some(5),
            ..Default::default()
        }),
        ["n1"]
    );
    // Historical reads use the same filters.
    assert_eq!(
        ids(&b
            .select(
                "r1",
                &FactFilter {
                    predicate: Some("p".into()),
                    ..Default::default()
                },
                10
            )
            .unwrap()),
        ["e1"]
    );
}

fn select_fails_instead_of_truncating(b: &mut dyn FactBackend) {
    two_revisions(b);
    assert_eq!(b.select("r2", &FactFilter::default(), 3).unwrap().len(), 3);
    assert_eq!(code(b.select("r2", &FactFilter::default(), 2)), "E_BUDGET");
    assert!(b
        .select(
            "r2",
            &FactFilter {
                predicate: Some("none".into()),
                ..Default::default()
            },
            0
        )
        .unwrap()
        .is_empty());
}

fn unknown_revisions_are_unavailable(b: &mut dyn FactBackend) {
    two_revisions(b);
    assert!(b.revision("missing").unwrap().is_none());
    assert!(!b.is_tip("missing").unwrap());
    assert_eq!(
        code(b.get("missing", Namespace::Node, "n1")),
        "E_UNAVAILABLE"
    );
    assert_eq!(
        code(b.select("missing", &FactFilter::default(), 10)),
        "E_UNAVAILABLE"
    );
    assert_eq!(code(b.header("missing")), "E_UNAVAILABLE");
    assert_eq!(code(b.ops("missing")), "E_UNAVAILABLE");
    assert_eq!(code(b.checkpoint("missing")), "E_UNAVAILABLE");
    assert_eq!(code(b.put_checkpoint("missing", "{}")), "E_UNAVAILABLE");
}

fn transactions_are_atomic_and_isolated(b: &mut dyn FactBackend) {
    assert_eq!(code(b.commit()), "E_INTEGRITY");
    assert_eq!(code(b.rollback()), "E_INTEGRITY");
    b.begin().unwrap();
    assert_eq!(code(b.begin()), "E_INTEGRITY");
    two_revisions(b);
    // The writer reads its own uncommitted writes.
    assert_eq!(all(b, "r2"), ["n1", "n2", "e2"]);
    b.put_checkpoint("r2", "{\"cached\":true}").unwrap();
    b.rollback().unwrap();
    assert!(b.revision("r1").unwrap().is_none() && b.revision("r2").unwrap().is_none());
    b.begin().unwrap();
    two_revisions(b);
    b.commit().unwrap();
    assert_eq!(all(b, "r2"), ["n1", "n2", "e2"]);
    // A rolled-back append after a commit leaves the committed state intact.
    b.begin().unwrap();
    b.append(
        rev(
            "r3",
            Some("r2"),
            30,
            vec![FactOp::Assert(node("x", "E", None))],
        ),
        None,
    )
    .unwrap();
    b.rollback().unwrap();
    assert!(b.revision("r3").unwrap().is_none() && b.is_tip("r2").unwrap());
    b.append(
        rev(
            "r3",
            Some("r2"),
            30,
            vec![FactOp::Assert(node("y", "E", None))],
        ),
        None,
    )
    .unwrap();
    assert_eq!(all(b, "r3"), ["n1", "n2", "y", "e2"]);
}

fn checkpoints_are_cached_and_droppable(b: &mut dyn FactBackend) {
    two_revisions(b);
    assert_eq!(b.checkpoint("r2").unwrap(), None);
    b.put_checkpoint("r2", "{\"a\":1}").unwrap();
    b.put_checkpoint("r2", "{\"a\":2}").unwrap();
    assert_eq!(b.checkpoint("r2").unwrap().as_deref(), Some("{\"a\":2}"));
    assert_eq!(b.checkpoint("r1").unwrap(), None);
    assert!(b.drop_checkpoint("r2").unwrap());
    assert!(!b.drop_checkpoint("r2").unwrap());
    assert_eq!(b.checkpoint("r2").unwrap(), None);
    // Checkpoints never change fact reads.
    assert_eq!(all(b, "r2"), ["n1", "n2", "e2"]);
}

fn parents_stay_in_their_graph(b: &mut dyn FactBackend) {
    two_revisions(b);
    let mut other = rev(
        "x1",
        Some("r2"),
        30,
        vec![FactOp::Assert(node("x", "E", None))],
    );
    other.graph_id = "other".into();
    assert_eq!(code(b.append(other.clone(), None)), "E_INTEGRITY");
    assert_eq!(code(b.append(other, empty_base())), "E_INTEGRITY");
}

fn malformed_input_is_rejected(b: &mut dyn FactBackend) {
    let one = |v: FactVersion| vec![FactOp::Assert(v)];
    assert_eq!(
        code(b.append(rev("", None, 1, one(node("n", "E", None))), empty_base())),
        "E_ID"
    );
    assert_eq!(
        code(b.append(rev("r", None, -1, one(node("n", "E", None))), empty_base())),
        "E_ID"
    );
    assert_eq!(
        code(b.append(
            rev("r", None, 1, one(node(&"x".repeat(513), "E", None))),
            empty_base()
        )),
        "E_ID"
    );
    assert_eq!(
        code(b.append(
            rev("r", None, 1, one(node("n", "E", Some((5, Some(5)))))),
            empty_base()
        )),
        "E_INTERVAL"
    );
    let mut big = node("n", "E", None);
    big.body = "x".repeat(MAX_BODY_BYTES + 1);
    assert_eq!(
        code(b.append(rev("r", None, 1, one(big)), empty_base())),
        "E_BUDGET"
    );
    let mut writer = rev("r", None, 1, one(node("n", "E", None)));
    writer.writer = String::new();
    assert_eq!(code(b.append(writer, empty_base())), "E_ID");
    assert!(b.revision("r").unwrap().is_none());
}

macro_rules! conformance {
    ($($name:ident),* $(,)?) => {
        mod memory_backend {
            use super::*;
            $( #[test] fn $name() { super::$name(&mut MemoryFactStore::new()); } )*
            #[test]
            fn is_not_durable() { assert!(!MemoryFactStore::new().durable()); }
        }
        mod sqlite_backend {
            use super::*;
            $( #[test] fn $name() {
                let connection = Connection::open_in_memory().unwrap();
                sqlite::install(&connection).unwrap();
                super::$name(&mut SqliteFactStore::new(&connection));
            } )*
        }
    };
}

conformance!(
    append_and_read_history,
    positions_are_contiguous_and_increasing,
    base_state_rules,
    fork_starts_independent_line,
    revision_ids_are_never_reused,
    operation_rules_and_atomic_failure,
    recorded_time_is_verbatim_and_monotonic,
    select_by_index_keys,
    select_fails_instead_of_truncating,
    unknown_revisions_are_unavailable,
    transactions_are_atomic_and_isolated,
    checkpoints_are_cached_and_droppable,
    parents_stay_in_their_graph,
    malformed_input_is_rejected,
);

/// Deterministic pseudo-random operation sequence, replayed on both backends.
#[test]
fn sqlite_matches_reference_backend_on_random_histories() {
    let connection = Connection::open_in_memory().unwrap();
    sqlite::install(&connection).unwrap();
    let mut sqlite_store = SqliteFactStore::new(&connection);
    let mut reference = MemoryFactStore::new();
    let mut seed: u64 = 0x5eed;
    let mut next = |bound: u64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) % bound
    };
    let mut revisions: Vec<String> = vec![];
    let mut rejected = 0;
    for step in 0..120u64 {
        let parent = if revisions.is_empty() || next(10) == 0 {
            None
        } else if next(6) == 0 {
            Some(revisions[next(revisions.len() as u64) as usize].clone())
        } else {
            revisions.last().cloned()
        };
        let continues = match &parent {
            Some(p) => reference.is_tip(p).unwrap(),
            None => false,
        };
        let base = (!continues).then(|| BaseState {
            header: format!("{{\"step\":{step}}}"),
            facts: match &parent {
                Some(p) => reference
                    .select(p, &FactFilter::default(), 10_000)
                    .unwrap()
                    .into_iter()
                    .map(|f| (f.version, f.recording))
                    .collect(),
                None => vec![],
            },
        });
        let live: Vec<(FactKind, String)> = match &parent {
            Some(p) => reference
                .select(p, &FactFilter::default(), 10_000)
                .unwrap()
                .into_iter()
                .map(|f| (f.version.kind, f.version.id))
                .collect(),
            None => vec![],
        };
        let mut ops = Vec::new();
        let mut used = std::collections::HashSet::new();
        for _ in 0..1 + next(4) {
            let roll = next(12);
            let op = if roll < 3 && !live.is_empty() {
                let (kind, id) = live[next(live.len() as u64) as usize].clone();
                FactOp::Retract { kind, id }
            } else {
                let id = format!("f{}", next(12));
                let kind = [FactKind::Node, FactKind::Edge, FactKind::Attachment][next(3) as usize];
                let start = next(100) as i64;
                match (roll, kind) {
                    // Occasionally an invalid retract, to compare error behaviour.
                    (3, _) => FactOp::Retract { kind, id },
                    (_, FactKind::Node) => FactOp::Assert(node(
                        &id,
                        &format!("E{}", next(4)),
                        (next(2) == 0).then_some((start, Some(start + 10))),
                    )),
                    (_, FactKind::Edge) => FactOp::Assert(edge(
                        &id,
                        &format!("f{}", next(5)),
                        &format!("f{}", next(5)),
                        ["p", "q"][next(2) as usize],
                        start,
                        (next(2) == 0).then_some(start + 20),
                    )),
                    _ => FactOp::Assert(attachment(&id, &format!("f{}", next(5)), "k")),
                }
            };
            if used.insert((op.kind().namespace(), op.id().to_string())) {
                ops.push(op);
            }
        }
        let new = NewRevision {
            revision: format!("r{step}"),
            graph_id: "g".into(),
            branch_id: "main".into(),
            parent,
            writer: format!("w{}", next(3)),
            recorded_at_ms: step as i64 * 10,
            ops,
        };
        let expected = reference.append(new.clone(), base.clone());
        let actual = sqlite_store.append(new, base);
        match (&expected, &actual) {
            (Ok(e), Ok(a)) => {
                assert_eq!(
                    (e.seq, e.op_count, &e.parent),
                    (a.seq, a.op_count, &a.parent)
                );
                revisions.push(e.revision.clone());
            }
            (Err(e), Err(a)) => {
                assert_eq!(e.code, a.code, "step {step}");
                rejected += 1;
            }
            _ => panic!("backends disagree at step {step}: {expected:?} vs {actual:?}"),
        }
        for revision in &revisions {
            let filters = [
                FactFilter::default(),
                FactFilter {
                    predicate: Some("p".into()),
                    ..Default::default()
                },
                FactFilter {
                    entity: Some("E1".into()),
                    ..Default::default()
                },
                FactFilter {
                    subject: Some("f1".into()),
                    ..Default::default()
                },
                FactFilter {
                    valid_at: Some(50),
                    ..Default::default()
                },
                FactFilter {
                    recorded: Some((0, step as i64 * 5)),
                    ..Default::default()
                },
            ];
            for filter in &filters {
                let mut e = reference.select(revision, filter, 10_000).unwrap();
                let mut a = sqlite_store.select(revision, filter, 10_000).unwrap();
                // Positions are backend-specific; everything else must agree.
                e.iter_mut().for_each(|f| f.position = 0);
                a.iter_mut().for_each(|f| f.position = 0);
                assert_eq!(e, a, "select {filter:?} at {revision}");
            }
            assert_eq!(
                reference.header(revision).unwrap(),
                sqlite_store.header(revision).unwrap()
            );
            assert_eq!(
                reference.is_tip(revision).unwrap(),
                sqlite_store.is_tip(revision).unwrap()
            );
            let strip = |ops: Vec<weave_engine::fact_store::LoggedOp>| {
                ops.into_iter().map(|o| o.op).collect::<Vec<_>>()
            };
            assert_eq!(
                strip(reference.ops(revision).unwrap()),
                strip(sqlite_store.ops(revision).unwrap())
            );
        }
    }
    assert!(
        revisions.len() > 60,
        "the sequence must exercise many revisions"
    );
    assert!(rejected > 0, "the sequence must exercise rejected appends");
}

#[test]
fn sqlite_commit_is_durable_and_uncommitted_writes_are_lost() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("facts.db");
    {
        let connection = Connection::open(&path).unwrap();
        sqlite::install(&connection).unwrap();
        let mut store = SqliteFactStore::new(&connection);
        assert!(store.durable());
        store.begin().unwrap();
        two_revisions(&mut store);
        store.commit().unwrap();
        store.begin().unwrap();
        store
            .append(
                rev(
                    "r3",
                    Some("r2"),
                    30,
                    vec![FactOp::Assert(node("x", "E", None))],
                ),
                None,
            )
            .unwrap();
        // The process ends without committing the second transaction.
    }
    let connection = Connection::open(&path).unwrap();
    sqlite::install(&connection).unwrap();
    let store = SqliteFactStore::new(&connection);
    assert_eq!(all(&store, "r2"), ["n1", "n2", "e2"]);
    assert!(store.revision("r3").unwrap().is_none());
    assert!(store.is_tip("r2").unwrap());
}

#[test]
fn sqlite_backend_joins_an_outer_transaction() {
    let connection = Connection::open_in_memory().unwrap();
    sqlite::install(&connection).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    {
        let mut store = SqliteFactStore::new(&connection);
        store.begin().unwrap();
        two_revisions(&mut store);
        store.commit().unwrap();
    }
    // Releasing the backend savepoint does not commit the outer transaction.
    connection.execute_batch("ROLLBACK").unwrap();
    let store = SqliteFactStore::new(&connection);
    assert!(store.revision("r1").unwrap().is_none());
}
