//! Local snapshot transport foundation. Hash integrity is not peer authenticity.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CapsuleRevision {
    pub graph_id: String,
    pub branch_id: String,
    pub revision: String,
    pub parent: Option<String>,
    pub data: GraphData,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Capsule {
    pub format: String,
    pub root: GraphRef,
    pub revisions: Vec<CapsuleRevision>,
    pub external_dependencies: Vec<GraphRef>,
}
impl CapsuleRevision {
    fn digest(&self) -> Result<String> {
        Ok(format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&(
                "weave-revision-v0.1",
                &self.graph_id,
                &self.branch_id,
                &self.parent,
                &self.data
            ))?)
        ))
    }
    fn dependencies(&self) -> Vec<GraphRef> {
        let mut dependencies = refs(&self.data);
        for edge in &self.data.edges {
            dependencies.extend(edge.derived_from.iter().map(|r| GraphRef {
                graph_id: r.graph_id.clone(),
                revision: r.revision.clone(),
            }));
        }
        if let Some(parent) = &self.parent {
            dependencies.push(GraphRef {
                graph_id: self.graph_id.clone(),
                revision: parent.clone(),
            });
        }
        dependencies
    }
}
impl Engine {
    fn revision_record(&self, reference: &GraphRef) -> Result<Option<CapsuleRevision>> {
        let row: Option<(String, Option<String>, String)> = self
            .conn
            .query_row(
                "SELECT branch_id,parent,data FROM revisions WHERE graph_id=?1 AND revision=?2",
                params![reference.graph_id, reference.revision],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        row.map(|(branch_id, parent, data)| {
            Ok(CapsuleRevision {
                graph_id: reference.graph_id.clone(),
                revision: reference.revision.clone(),
                branch_id,
                parent,
                data: serde_json::from_str(&data)?,
            })
        })
        .transpose()
    }
    /// Export an exact authorized root with bounded ancestry and metadata dependencies.
    /// A hidden dependency is declared external, never exported as a filtered fake revision.
    pub fn export_capsule(&self, root: &GraphRef, host: &HostContext) -> Result<Capsule> {
        let mut pending = std::collections::VecDeque::from([(root.clone(), 0usize)]);
        let mut seen = HashSet::new();
        let mut revisions = Vec::new();
        let mut export_bytes = 0usize;
        let mut external_dependencies = Vec::new();
        while let Some((reference, depth)) = pending.pop_front() {
            if !seen.insert((reference.graph_id.clone(), reference.revision.clone())) {
                continue;
            }
            if seen.len() > 1000 || depth > 32 {
                external_dependencies.push(reference);
                continue;
            }
            let Some(record) = self.revision_record(&reference)? else {
                if &reference == root {
                    return Err(err("E_UNAVAILABLE", "capsule root unavailable"));
                }
                external_dependencies.push(reference);
                continue;
            };
            let (visible, incomplete) = self.authorized(record.data.clone(), host)?;
            if visible != record.data || incomplete {
                if &reference == root {
                    return Err(err("E_UNAVAILABLE", "capsule root unavailable"));
                }
                external_dependencies.push(reference);
                continue;
            }
            pending.extend(record.dependencies().into_iter().map(|r| (r, depth + 1)));
            export_bytes += json_size(
                &record,
                (16usize * 1024 * 1024).saturating_sub(export_bytes),
            )?;
            revisions.push(record);
        }
        let capsule = Capsule {
            format: "weave-capsule-0.1".into(),
            root: root.clone(),
            revisions,
            external_dependencies,
        };
        if json_size(&capsule, 16 * 1024 * 1024).is_err() {
            return Err(err("E_BUDGET", "capsule exceeds 16 MiB"));
        }
        Ok(capsule)
    }
    /// Store verified immutable revisions without changing accepted branch heads or events.
    /// Host graph write grants authorize storage here; they are not received from capsule JSON.
    pub fn receive_capsule(&mut self, capsule: &Capsule, host: &HostContext) -> Result<usize> {
        if capsule.format != "weave-capsule-0.1" {
            return Err(err("E_VERSION", "unsupported capsule format"));
        }
        if capsule.revisions.len() > 1000 || json_size(capsule, 16 * 1024 * 1024).is_err() {
            return Err(err("E_BUDGET", "capsule budget exceeded"));
        }
        let mut included = HashSet::new();
        for record in &capsule.revisions {
            if !host.writable_graphs.contains(&record.graph_id) {
                return Err(err(
                    "E_FORBIDDEN",
                    "host has not granted capsule storage authority",
                ));
            }
            validate_graph(&record.data)?;
            if !valid_id(&record.graph_id)
                || !valid_id(&record.branch_id)
                || record.digest()? != record.revision
            {
                return Err(err("E_INTEGRITY", "capsule revision digest mismatch"));
            }
            if !included.insert((record.graph_id.clone(), record.revision.clone())) {
                return Err(err("E_INTEGRITY", "duplicate capsule revision"));
            }
        }
        if !included.contains(&(capsule.root.graph_id.clone(), capsule.root.revision.clone())) {
            return Err(err("E_INTEGRITY", "capsule root absent"));
        }
        let external: HashSet<_> = capsule
            .external_dependencies
            .iter()
            .map(|r| (r.graph_id.clone(), r.revision.clone()))
            .collect();
        for record in &capsule.revisions {
            for dependency in record.dependencies() {
                let key = (dependency.graph_id, dependency.revision);
                if !included.contains(&key) && !external.contains(&key) {
                    return Err(err("E_INTEGRITY", "undeclared external dependency"));
                }
            }
        }
        let tx = self.conn.transaction()?;
        let mut inserted = 0;
        let recorded_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| err("E_CLOCK", "clock before epoch"))?
            .as_millis();
        let recorded_at =
            i64::try_from(recorded_at).map_err(|_| err("E_CLOCK", "clock out of range"))?;
        for record in &capsule.revisions {
            inserted += tx.execute(
                "INSERT OR IGNORE INTO revisions VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    record.revision,
                    record.graph_id,
                    record.branch_id,
                    record.parent,
                    recorded_at,
                    serde_json::to_string(&record.data)?
                ],
            )?;
        }
        tx.commit()?;
        Ok(inserted)
    }
    /// Explicitly accept a received/local revision into a branch using strict CAS.
    /// This is owner publication, not collective governance or conflict resolution.
    pub fn accept_revision(
        &mut self,
        reference: &GraphRef,
        branch: &str,
        expected: Option<&str>,
        host: &HostContext,
    ) -> Result<()> {
        if !valid_id(branch) || !valid_id(&host.principal) {
            return Err(err("E_ID", "branch and principal required"));
        }
        if !host.writable_graphs.contains(&reference.graph_id) {
            return Err(err(
                "E_FORBIDDEN",
                "host has not granted acceptance authority",
            ));
        }
        let tx = self.conn.unchecked_transaction()?;
        let head = self.head(&reference.graph_id, branch)?;
        if head.as_deref() != expected {
            return Err(err(
                "E_CONFLICT",
                "expected head differs from current branch head",
            ));
        }
        let record = self
            .revision_record(reference)?
            .ok_or_else(|| err("E_UNAVAILABLE", "revision unavailable"))?;
        let (visible, incomplete) = self.authorized(record.data.clone(), host)?;
        if visible != record.data || incomplete {
            return Err(err("E_UNAVAILABLE", "revision unavailable"));
        }
        if head.as_ref() == Some(&reference.revision) {
            tx.commit()?;
            return Ok(());
        }
        let event_id = format!(
            "accept:{:x}",
            Sha256::digest(serde_json::to_vec(&(
                &reference.graph_id,
                branch,
                &head,
                &reference.revision,
                self.event_count()?
            ))?)
        );
        tx.execute("INSERT INTO heads VALUES (?1,?2,?3) ON CONFLICT(graph_id,branch_id) DO UPDATE SET revision=excluded.revision",params![reference.graph_id,branch,reference.revision])?;
        tx.execute("INSERT INTO events(event_id,graph_id,branch_id,revision,actor) VALUES (?1,?2,?3,?4,?5)",params![event_id,reference.graph_id,branch,reference.revision,host.principal])?;
        tx.commit()?;
        Ok(())
    }
    /// Create an independent branch at an immutable revision. Existing branches reject.
    pub fn fork_branch(
        &mut self,
        reference: &GraphRef,
        branch: &str,
        host: &HostContext,
    ) -> Result<()> {
        self.accept_revision(reference, branch, None, host)
    }
}
