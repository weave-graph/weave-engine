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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub manifests: Vec<SnapshotManifest>,
}
impl CapsuleRevision {
    pub(crate) fn digest(&self) -> Result<String> {
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
        for premises in self
            .data
            .edges
            .iter()
            .map(|e| &e.derived_from)
            .chain(self.data.assertions.iter().map(|a| &a.derived_from))
        {
            dependencies.extend(premises.iter().map(|r| GraphRef {
                graph_id: r.graph_id.clone(),
                revision: r.revision.clone(),
            }));
        }
        dependencies.extend(
            self.data
                .attachments
                .iter()
                .filter_map(|a| a.origin.as_ref())
                .map(|r| GraphRef {
                    graph_id: r.graph_id.clone(),
                    revision: r.revision.clone(),
                }),
        );
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
    pub(crate) fn revision_record(&self, reference: &GraphRef) -> Result<Option<CapsuleRevision>> {
        let Some(data) = self.load(&reference.graph_id, &reference.revision)? else {
            return Ok(None);
        };
        let (branch_id,parent):(String,Option<String>)=self.conn.query_row("SELECT substr(branch_id,1,513),substr(parent,1,513) FROM revisions WHERE graph_id=?1 AND revision=?2",params![reference.graph_id,reference.revision],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if !valid_id(&branch_id) || parent.as_ref().is_some_and(|p| !valid_id(p)) {
            return Err(err("E_INTEGRITY", "stored revision identifiers invalid"));
        }
        Ok(Some(CapsuleRevision {
            graph_id: reference.graph_id.clone(),
            revision: reference.revision.clone(),
            branch_id,
            parent,
            data,
        }))
    }
    /// Export exact authorized snapshots. Logical snapshots include their whole manifest.
    /// Hidden manifest members cannot be disclosed through hashes or membership lists.
    pub fn export_capsule(&self, root: &GraphRef, host: &HostContext) -> Result<Capsule> {
        self.export_capsule_profile(root, host, false, &mut 0)
    }
    pub(crate) fn export_capsule_profile(
        &self,
        root: &GraphRef,
        host: &HostContext,
        complete: bool,
        work: &mut usize,
    ) -> Result<Capsule> {
        let _snapshot = self.optional_read_transaction()?;
        let _clock_scope = self.operation_scope()?;
        let _read_scope = self.read_budget.enter();
        let mut pending = std::collections::VecDeque::from([(root.clone(), 0usize)]);
        let mut queued = HashSet::from([(root.graph_id.clone(), root.revision.clone())]);
        let mut seen = HashSet::new();
        let mut included = HashSet::new();
        let mut revisions = Vec::new();
        let mut manifests = BTreeMap::new();
        let mut export_bytes = 0usize;
        let mut external_dependencies = Vec::new();
        'pending: while let Some((reference, depth)) = pending.pop_front() {
            let key = (reference.graph_id.clone(), reference.revision.clone());
            if included.contains(&key) || !seen.insert(key) {
                continue;
            }
            if seen.len() > 1000 || depth > 32 {
                if complete {
                    return Err(err("E_BUDGET", "export closure exceeds budget"));
                }
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
            let mut group = vec![record];
            let mut manifest = None;
            if reference.revision.starts_with("logical:") {
                let id: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT manifest_id FROM revision_integrity WHERE revision=?1",
                        [&reference.revision],
                        |r| r.get(0),
                    )
                    .optional()?;
                let id =
                    id.ok_or_else(|| err("E_INTEGRITY", "logical snapshot manifest unavailable"))?;
                let source = self
                    .snapshot_manifest(&id)?
                    .ok_or_else(|| err("E_INTEGRITY", "logical snapshot manifest unavailable"))?;
                group.clear();
                // Authorize one record at a time before any size-dependent response.
                // Then charge the group incrementally before retaining its records.
                for member in &source.members {
                    let member_ref = GraphRef {
                        graph_id: member.graph_id.clone(),
                        revision: member.revision.clone(),
                    };
                    let row = self
                        .revision_record(&member_ref)?
                        .ok_or_else(|| err("E_INTEGRITY", "logical snapshot member unavailable"))?;
                    let (visible, incomplete) = self.authorized(row.data.clone(), host)?;
                    if !whole_graph_visible(&row.data, visible, incomplete)
                        || !self.protected_reference_allowed(&row.graph_id, &row.revision, host)?
                    {
                        if &reference == root {
                            return Err(err("E_UNAVAILABLE", "capsule root unavailable"));
                        }
                        external_dependencies.push(reference);
                        continue 'pending;
                    }
                }
                let mut group_bytes = export_bytes
                    + json_size(
                        &source,
                        (16usize * 1024 * 1024).saturating_sub(export_bytes),
                    )?;
                for member in &source.members {
                    let member_ref = GraphRef {
                        graph_id: member.graph_id.clone(),
                        revision: member.revision.clone(),
                    };
                    let row = self
                        .revision_record(&member_ref)?
                        .ok_or_else(|| err("E_INTEGRITY", "logical snapshot member unavailable"))?;
                    group_bytes +=
                        json_size(&row, (16usize * 1024 * 1024).saturating_sub(group_bytes))?;
                    group.push(row);
                }
                if verify_manifest(&source, &group)? != id {
                    return Err(err("E_INTEGRITY", "stored manifest digest mismatch"));
                }
                manifest = Some((id, source));
            }
            let mut available = true;
            let mut live = false;
            for row in &group {
                if !row.revision.starts_with("logical:") && row.digest()? != row.revision {
                    return Err(err("E_INTEGRITY", "stored revision digest mismatch"));
                }
                live |= row
                    .data
                    .attachments
                    .iter()
                    .any(|a| matches!(a.value, MetadataValue::LiveGraph { .. }));
                let (visible, incomplete) = self.authorized(row.data.clone(), host)?;
                available &= whole_graph_visible(&row.data, visible, incomplete)
                    && self.protected_reference_allowed(&row.graph_id, &row.revision, host)?;
            }
            if !available || live {
                if &reference == root {
                    return Err(if !available {
                        err("E_UNAVAILABLE", "capsule root unavailable")
                    } else {
                        err(
                            "E_CAPSULE_VERSION",
                            "live handles require a pinned export context",
                        )
                    });
                }
                external_dependencies.push(reference);
                continue;
            }
            let fresh = group
                .iter()
                .filter(|r| !included.contains(&(r.graph_id.clone(), r.revision.clone())))
                .count();
            if included.len() + fresh > 1000 {
                return Err(err("E_BUDGET", "capsule exceeds 1000 snapshots"));
            }
            if let Some((id, value)) = manifest {
                if let std::collections::btree_map::Entry::Vacant(entry) = manifests.entry(id) {
                    export_bytes +=
                        json_size(&value, (16usize * 1024 * 1024).saturating_sub(export_bytes))?;
                    entry.insert(value);
                }
            }
            for row in group {
                if !included.insert((row.graph_id.clone(), row.revision.clone())) {
                    continue;
                }
                if complete {
                    capsule_export::visit_dependencies(&row, work, |graph, revision| {
                        let key = (graph.to_owned(), revision.to_owned());
                        if !queued.contains(&key) {
                            if queued.len() >= 1000 {
                                return Err(err("E_BUDGET", "export queue exceeds budget"));
                            }
                            queued.insert(key);
                            pending.push_back((
                                GraphRef {
                                    graph_id: graph.into(),
                                    revision: revision.into(),
                                },
                                depth + 1,
                            ));
                        }
                        Ok(())
                    })?;
                } else {
                    pending.extend(row.dependencies().into_iter().map(|r| (r, depth + 1)));
                }
                export_bytes +=
                    json_size(&row, (16usize * 1024 * 1024).saturating_sub(export_bytes))?;
                revisions.push(row);
            }
        }
        external_dependencies
            .retain(|r| !included.contains(&(r.graph_id.clone(), r.revision.clone())));
        for record in &revisions {
            for attachment in record.data.attachments.iter().filter(|a| a.required) {
                if let MetadataValue::Graph { reference } = &attachment.value {
                    if !included.contains(&(reference.graph_id.clone(), reference.revision.clone()))
                    {
                        return Err(err(
                            "E_DEPENDENCY_UNAVAILABLE",
                            "required metadata cannot be included in this capsule",
                        ));
                    }
                }
            }
        }
        let capsule = Capsule {
            format: if revisions
                .iter()
                .any(|r| weave_contract::carrier_profile::requires_v019(&r.data))
            {
                "weave-capsule-0.4"
            } else if revisions.iter().any(|r| influence::requires_v017(&r.data)) {
                "weave-capsule-0.3"
            } else if manifests.is_empty() {
                "weave-capsule-0.1"
            } else {
                "weave-capsule-0.2"
            }
            .into(),
            root: root.clone(),
            revisions,
            external_dependencies,
            manifests: manifests.into_values().collect(),
        };
        json_size(&capsule, 16 * 1024 * 1024)?;
        Ok(capsule)
    }
    /// Verify and quarantine snapshots. Receipt never advances accepted heads or emits events.
    /// Hashes authenticate byte consistency only, not a peer or an assertion's truth.
    pub fn receive_capsule(&mut self, capsule: &Capsule, host: &HostContext) -> Result<usize> {
        self.conn.execute_batch("SAVEPOINT capsule_receive")?;
        let result = (|| {
            let _clock_scope = self.operation_write_scope()?;
            self.receive_capsule_inner(capsule, host)
        })();
        match result {
            Ok(value) => {
                self.conn.execute_batch("RELEASE capsule_receive")?;
                Ok(value)
            }
            Err(error) => {
                self.conn
                    .execute_batch("ROLLBACK TO capsule_receive; RELEASE capsule_receive")?;
                Err(error)
            }
        }
    }
    fn receive_capsule_inner(&self, capsule: &Capsule, host: &HostContext) -> Result<usize> {
        let _read_scope = self.read_budget.enter();
        if ![
            "weave-capsule-0.1",
            "weave-capsule-0.2",
            "weave-capsule-0.3",
            "weave-capsule-0.4",
        ]
        .contains(&capsule.format.as_str())
        {
            return Err(err("E_VERSION", "unsupported capsule format"));
        }
        if capsule.format != "weave-capsule-0.4"
            && capsule
                .revisions
                .iter()
                .any(|r| weave_contract::carrier_profile::requires_v019(&r.data))
        {
            return Err(err("E_VERSION", "alternative carriers require capsule 0.4"));
        }
        if !["weave-capsule-0.3", "weave-capsule-0.4"].contains(&capsule.format.as_str())
            && capsule
                .revisions
                .iter()
                .any(|r| influence::requires_v017(&r.data))
        {
            return Err(err("E_VERSION", "snapshot influence requires capsule 0.3"));
        }
        if capsule.format == "weave-capsule-0.1" && !capsule.manifests.is_empty() {
            return Err(err("E_VERSION", "logical manifests require capsule 0.2"));
        }
        if capsule.revisions.len() > 1000
            || capsule.manifests.len() > 1000
            || json_size(capsule, 16 * 1024 * 1024).is_err()
        {
            return Err(err("E_BUDGET", "capsule budget exceeded"));
        }
        for record in &capsule.revisions {
            identity_acceptance::require_external_graph(&record.graph_id)?;
            identity_acceptance::require_external_schema(&record.data)?;
        }
        let mut manifest_ids = BTreeMap::new();
        let mut logical = BTreeMap::new();
        for manifest in &capsule.manifests {
            let id = verify_manifest(manifest, &capsule.revisions)?;
            if manifest_ids
                .insert(manifest.batch_id.clone(), id.clone())
                .is_some()
            {
                return Err(err("E_INTEGRITY", "duplicate capsule batch identity"));
            }
            for member in &manifest.members {
                if logical
                    .insert(
                        member.revision.clone(),
                        (member.content_digest.clone(), id.clone()),
                    )
                    .is_some()
                {
                    return Err(err(
                        "E_INTEGRITY",
                        "logical revision occurs in multiple manifests",
                    ));
                }
            }
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
                || !valid_id(&record.revision)
                || record
                    .parent
                    .as_ref()
                    .is_some_and(|p| !valid_id(p) || p == &record.revision)
            {
                return Err(err("E_INTEGRITY", "capsule identifiers invalid"));
            }
            if record
                .data
                .attachments
                .iter()
                .any(|a| matches!(a.value, MetadataValue::LiveGraph { .. }))
            {
                return Err(err(
                    "E_CAPSULE_VERSION",
                    "live handles require a pinned export context",
                ));
            }
            let digest = record.digest()?;
            if record.revision.starts_with("logical:") {
                if ![
                    "weave-capsule-0.2",
                    "weave-capsule-0.3",
                    "weave-capsule-0.4",
                ]
                .contains(&capsule.format.as_str())
                    || logical
                        .get(&record.revision)
                        .is_none_or(|(expected, _)| expected != &digest)
                {
                    return Err(err(
                        "E_INTEGRITY",
                        "logical revision lacks matching manifest proof",
                    ));
                }
            } else if digest != record.revision {
                return Err(err("E_INTEGRITY", "capsule revision digest mismatch"));
            }
            if !included.insert((record.graph_id.clone(), record.revision.clone())) {
                return Err(err("E_INTEGRITY", "duplicate capsule revision"));
            }
        }
        if !included.contains(&(capsule.root.graph_id.clone(), capsule.root.revision.clone())) {
            return Err(err("E_INTEGRITY", "capsule root absent"));
        }
        let mut external = HashSet::new();
        for reference in &capsule.external_dependencies {
            let key = (reference.graph_id.clone(), reference.revision.clone());
            if !valid_id(&reference.graph_id)
                || !valid_id(&reference.revision)
                || included.contains(&key)
                || !external.insert(key)
            {
                return Err(err(
                    "E_INTEGRITY",
                    "invalid or duplicate external dependency",
                ));
            }
        }
        for record in &capsule.revisions {
            for dependency in record.dependencies() {
                let key = (dependency.graph_id, dependency.revision);
                if !included.contains(&key) && !external.contains(&key) {
                    return Err(err("E_INTEGRITY", "undeclared external dependency"));
                }
            }
        }
        // Metadata cycles are meaningful; revision ancestry must stay acyclic.
        let parents: BTreeMap<_, _> = capsule
            .revisions
            .iter()
            .map(|r| {
                (
                    (r.graph_id.as_str(), r.revision.as_str()),
                    r.parent.as_deref(),
                )
            })
            .collect();
        for record in &capsule.revisions {
            let mut visited = HashSet::new();
            let mut cursor = Some(record.revision.clone());
            while let Some(revision) = cursor {
                if !visited.insert(revision.clone()) {
                    return Err(err("E_INTEGRITY", "revision ancestry cycle"));
                }
                if visited.len() > 1000 {
                    return Err(err(
                        "E_BUDGET",
                        "capsule ancestry validation exceeds 1000 revisions",
                    ));
                }
                cursor = if let Some(parent) =
                    parents.get(&(record.graph_id.as_str(), revision.as_str()))
                {
                    parent.map(String::from)
                } else {
                    self.conn
                        .query_row(
                            "SELECT parent FROM revisions WHERE graph_id=?1 AND revision=?2",
                            params![record.graph_id, revision],
                            |r| r.get::<_, Option<String>>(0),
                        )
                        .optional()?
                        .flatten()
                };
            }
        }
        let tx = &self.conn;
        for manifest in &capsule.manifests {
            let id = &manifest_ids[&manifest.batch_id];
            let existing: Option<String> = tx
                .query_row(
                    "SELECT id FROM snapshot_manifests WHERE batch_id=?1",
                    [&manifest.batch_id],
                    |r| r.get(0),
                )
                .optional()?;
            if existing.as_ref().is_some_and(|old| old != id) {
                return Err(err(
                    "E_EQUIVOCATION",
                    "batch identity already binds another manifest",
                ));
            }
            tx.execute(
                "INSERT OR IGNORE INTO snapshot_manifests VALUES (?1,?2,?3)",
                params![id, manifest.batch_id, serde_json::to_string(manifest)?],
            )?;
        }
        let recorded_at = self.operation_time()?;
        let mut inserted = 0;
        for record in &capsule.revisions {
            self.validate_structures(&record.graph_id, &record.data)?;
            if let Some(existing) = self.revision_record(&GraphRef {
                graph_id: record.graph_id.clone(),
                revision: record.revision.clone(),
            })? {
                if existing != *record {
                    return Err(err(
                        "E_EQUIVOCATION",
                        "revision identity already binds another snapshot",
                    ));
                }
            }
            self.record_structures(&record.graph_id, &record.data)?;
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
            if let Some((digest, manifest)) = logical.get(&record.revision) {
                let previous:Option<(String,String)>=tx.query_row("SELECT content_digest,manifest_id FROM revision_integrity WHERE revision=?1",[&record.revision],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                if previous.is_some_and(|p| p != (digest.clone(), manifest.clone())) {
                    return Err(err("E_EQUIVOCATION", "logical revision proof changed"));
                }
                tx.execute(
                    "INSERT OR IGNORE INTO revision_integrity VALUES (?1,?2,?3)",
                    params![record.revision, digest, manifest],
                )?;
            }
        }
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
        self.conn.execute_batch("SAVEPOINT capsule_accept")?;
        let result = (|| {
            let _clock_scope = self.operation_write_scope()?;
            self.accept_revision_inner(reference, branch, expected, host)
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("RELEASE capsule_accept")?;
                Ok(())
            }
            Err(error) => {
                self.conn
                    .execute_batch("ROLLBACK TO capsule_accept; RELEASE capsule_accept")?;
                Err(error)
            }
        }
    }
    fn accept_revision_inner(
        &self,
        reference: &GraphRef,
        branch: &str,
        expected: Option<&str>,
        host: &HostContext,
    ) -> Result<()> {
        let _read_scope = self.read_budget.enter();
        identity_acceptance::require_external_graph(&reference.graph_id)?;
        if !valid_id(branch) || !valid_id(&host.principal) {
            return Err(err("E_ID", "branch and principal required"));
        }
        if !host.writable_graphs.contains(&reference.graph_id) {
            return Err(err(
                "E_FORBIDDEN",
                "host has not granted acceptance authority",
            ));
        }
        let tx = &self.conn;
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
        if !whole_graph_visible(&record.data, visible, incomplete) {
            return Err(err("E_UNAVAILABLE", "revision unavailable"));
        }
        self.validate_required_metadata(&record.data, host)?;
        if head.as_ref() == Some(&reference.revision) {
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

/// Verify canonical whole-manifest membership and each record's content binding.
pub(crate) fn verify_manifest(
    manifest: &SnapshotManifest,
    records: &[CapsuleRevision],
) -> Result<String> {
    if manifest.batch_id.is_empty()
        || manifest.batch_id.len() > 64
        || !manifest
            .batch_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        || manifest.members.is_empty()
        || manifest.members.len() > 100
    {
        return Err(err("E_INTEGRITY", "invalid snapshot manifest"));
    }
    let mut previous: Option<&str> = None;
    for member in &manifest.members {
        if previous.is_some_and(|p| p >= member.graph_id.as_str())
            || member.revision != format!("logical:{}:{}", manifest.batch_id, member.graph_id)
        {
            return Err(err("E_INTEGRITY", "manifest membership is not canonical"));
        }
        previous = Some(&member.graph_id);
        let record = records
            .iter()
            .find(|r| r.graph_id == member.graph_id && r.revision == member.revision)
            .ok_or_else(|| err("E_INTEGRITY", "whole manifest membership is required"))?;
        if record.branch_id != member.branch_id
            || record.parent != member.parent
            || record.digest()? != member.content_digest
        {
            return Err(err("E_INTEGRITY", "manifest member content mismatch"));
        }
    }
    Ok(format!(
        "manifest:{:x}",
        Sha256::digest(serde_json::to_vec(&("weave-manifest-v0.4", manifest))?)
    ))
}
