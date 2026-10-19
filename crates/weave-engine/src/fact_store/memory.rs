//! Non-durable reference backend. It defines the trait's semantics with the
//! simplest possible data structures, so the SQLite backend can be checked
//! against it by the conformance suite.
use super::{
    order, validate_new, BaseState, FactBackend, FactFilter, FactOp, FactVersion, LoggedOp,
    Namespace, NewRevision, Recording, RevisionInfo, StoredFact,
};
use crate::{err, Result};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
struct Line {
    graph_id: String,
    header: String,
    tip: i64,
}

#[derive(Debug, Clone)]
struct Version {
    line: i64,
    from_seq: i64,
    until_seq: Option<i64>,
    fact: StoredFact,
}

#[derive(Debug, Clone, Default)]
struct State {
    next_position: i64,
    lines: BTreeMap<i64, Line>,
    revisions: BTreeMap<String, RevisionInfo>,
    /// Every logged operation by position; base entries have no revision operation.
    log: BTreeMap<i64, FactOp>,
    versions: Vec<Version>,
    checkpoints: BTreeMap<String, String>,
}

/// In-memory [`FactBackend`]. `durable()` is false.
#[derive(Debug, Default)]
pub struct MemoryFactStore {
    state: State,
    saved: Option<State>,
}

impl MemoryFactStore {
    pub fn new() -> Self {
        Self::default()
    }
}

fn unavailable() -> crate::Error {
    err("E_UNAVAILABLE", "fact revision unavailable")
}

impl State {
    fn info(&self, revision: &str) -> Result<&RevisionInfo> {
        self.revisions.get(revision).ok_or_else(unavailable)
    }
    fn live_at<'a>(&'a self, line: i64, seq: i64) -> impl Iterator<Item = &'a Version> + 'a {
        self.versions.iter().filter(move |v| {
            v.line == line && v.from_seq <= seq && v.until_seq.is_none_or(|until| until > seq)
        })
    }
    fn live_index(&self, line: i64, seq: i64, namespace: Namespace, id: &str) -> Option<usize> {
        self.versions.iter().position(|v| {
            v.line == line
                && v.from_seq <= seq
                && v.until_seq.is_none_or(|until| until > seq)
                && v.fact.version.kind.namespace() == namespace
                && v.fact.version.id == id
        })
    }
    fn push_version(
        &mut self,
        line: i64,
        seq: i64,
        version: FactVersion,
        recording: Recording,
    ) -> i64 {
        let position = self.next_position;
        self.next_position += 1;
        self.versions.push(Version {
            line,
            from_seq: seq,
            until_seq: None,
            fact: StoredFact {
                position,
                version,
                recording,
            },
        });
        position
    }
    fn append(&mut self, revision: NewRevision, base: Option<BaseState>) -> Result<RevisionInfo> {
        validate_new(&revision, base.as_ref())?;
        if self.revisions.contains_key(&revision.revision) {
            return Err(err("E_CONFLICT", "fact revision already exists"));
        }
        if self.next_position == 0 {
            self.next_position = 1;
        }
        let parent = match &revision.parent {
            Some(parent) => self.revisions.get(parent).cloned(),
            None => None,
        };
        if let Some(parent) = &parent {
            if parent.graph_id != revision.graph_id {
                return Err(err("E_INTEGRITY", "fact parent belongs to another graph"));
            }
        }
        let continues = parent
            .as_ref()
            .is_some_and(|p| self.lines.get(&p.line).is_some_and(|l| l.tip == p.seq));
        let (line, seq) = if continues {
            let parent = parent.as_ref().ok_or_else(unavailable)?;
            if base.is_some() {
                return Err(err(
                    "E_INTEGRITY",
                    "a line continuation takes no base state",
                ));
            }
            if revision.recorded_at_ms < parent.recorded_at_ms {
                return Err(err("E_HISTORY_CLOCK", "recorded time regressed on a line"));
            }
            (parent.line, parent.seq + 1)
        } else {
            let base =
                base.ok_or_else(|| err("E_INTEGRITY", "a new line requires a base state"))?;
            if base
                .facts
                .iter()
                .any(|(_, r)| r.recorded_at_ms > revision.recorded_at_ms)
            {
                return Err(err("E_HISTORY_CLOCK", "recorded time regressed on a line"));
            }
            let line = self.lines.keys().next_back().map_or(1, |l| l + 1);
            self.lines.insert(
                line,
                Line {
                    graph_id: revision.graph_id.clone(),
                    header: base.header,
                    tip: 0,
                },
            );
            for (version, recording) in base.facts {
                self.push_version(line, 0, version, recording);
            }
            (line, 1)
        };
        let recording = Recording {
            writer: revision.writer.clone(),
            on_behalf_of: revision.on_behalf_of.clone(),
            recorded_at_ms: revision.recorded_at_ms,
            revision: revision.revision.clone(),
        };
        let first_position = self.next_position;
        for op in &revision.ops {
            let live = self.live_index(line, seq - 1, op.kind().namespace(), op.id());
            if let Some(index) = live {
                if self.versions[index].fact.version.kind != op.kind() {
                    return Err(err("E_CONFLICT", "fact kind differs from the live fact"));
                }
                self.versions[index].until_seq = Some(seq);
            }
            match op {
                FactOp::Assert(version) => {
                    let position = self.push_version(line, seq, version.clone(), recording.clone());
                    self.log.insert(position, op.clone());
                }
                FactOp::Retract { .. } => {
                    if live.is_none() {
                        return Err(err("E_CONFLICT", "retracted fact is not live"));
                    }
                    let position = self.next_position;
                    self.next_position += 1;
                    self.log.insert(position, op.clone());
                }
            }
        }
        let line_state = self.lines.get_mut(&line).ok_or_else(unavailable)?;
        if line_state.graph_id != revision.graph_id {
            return Err(err("E_INTEGRITY", "fact line belongs to another graph"));
        }
        line_state.tip = seq;
        let info = RevisionInfo {
            revision: revision.revision.clone(),
            graph_id: revision.graph_id,
            branch_id: revision.branch_id,
            parent: revision.parent,
            line,
            seq,
            first_position,
            op_count: revision.ops.len() as i64,
            writer: revision.writer,
            on_behalf_of: revision.on_behalf_of,
            recorded_at_ms: revision.recorded_at_ms,
        };
        self.revisions.insert(revision.revision, info.clone());
        Ok(info)
    }
}

impl FactBackend for MemoryFactStore {
    fn durable(&self) -> bool {
        false
    }
    fn begin(&mut self) -> Result<()> {
        if self.saved.is_some() {
            return Err(err("E_INTEGRITY", "fact transaction already open"));
        }
        self.saved = Some(self.state.clone());
        Ok(())
    }
    fn commit(&mut self) -> Result<()> {
        self.saved
            .take()
            .map(|_| ())
            .ok_or_else(|| err("E_INTEGRITY", "no fact transaction open"))
    }
    fn rollback(&mut self) -> Result<()> {
        self.state = self
            .saved
            .take()
            .ok_or_else(|| err("E_INTEGRITY", "no fact transaction open"))?;
        Ok(())
    }
    fn revision(&self, revision: &str) -> Result<Option<RevisionInfo>> {
        Ok(self.state.revisions.get(revision).cloned())
    }
    fn is_tip(&self, revision: &str) -> Result<bool> {
        Ok(self.state.revisions.get(revision).is_some_and(|info| {
            self.state
                .lines
                .get(&info.line)
                .is_some_and(|line| line.tip == info.seq)
        }))
    }
    fn append(&mut self, revision: NewRevision, base: Option<BaseState>) -> Result<RevisionInfo> {
        // Apply to a copy so a failed append changes nothing.
        let mut next = self.state.clone();
        let info = next.append(revision, base)?;
        self.state = next;
        Ok(info)
    }
    fn header(&self, revision: &str) -> Result<String> {
        let info = self.state.info(revision)?;
        Ok(self
            .state
            .lines
            .get(&info.line)
            .ok_or_else(unavailable)?
            .header
            .clone())
    }
    fn get(&self, revision: &str, namespace: Namespace, id: &str) -> Result<Option<StoredFact>> {
        let info = self.state.info(revision)?;
        Ok(self
            .state
            .live_index(info.line, info.seq, namespace, id)
            .map(|index| self.state.versions[index].fact.clone()))
    }
    fn select(&self, revision: &str, filter: &FactFilter, limit: usize) -> Result<Vec<StoredFact>> {
        let info = self.state.info(revision)?;
        let mut facts: Vec<StoredFact> = self
            .state
            .live_at(info.line, info.seq)
            .filter(|v| filter.matches(&v.fact))
            .map(|v| v.fact.clone())
            .collect();
        if facts.len() > limit {
            return Err(err("E_BUDGET", "fact selection exceeds its limit"));
        }
        order(&mut facts);
        Ok(facts)
    }
    fn ops(&self, revision: &str) -> Result<Vec<LoggedOp>> {
        let info = self.state.info(revision)?;
        Ok(self
            .state
            .log
            .range(info.first_position..info.first_position + info.op_count)
            .map(|(position, op)| LoggedOp {
                position: *position,
                op: op.clone(),
            })
            .collect())
    }
    fn put_checkpoint(&mut self, revision: &str, body: &str) -> Result<()> {
        self.state.info(revision)?;
        if body.len() > super::MAX_CHECKPOINT_BYTES {
            return Err(err("E_BUDGET", "checkpoint exceeds 256 MiB"));
        }
        self.state.checkpoints.insert(revision.into(), body.into());
        Ok(())
    }
    fn checkpoint(&self, revision: &str) -> Result<Option<String>> {
        self.state.info(revision)?;
        Ok(self.state.checkpoints.get(revision).cloned())
    }
    fn drop_checkpoint(&mut self, revision: &str) -> Result<bool> {
        Ok(self.state.checkpoints.remove(revision).is_some())
    }
}
