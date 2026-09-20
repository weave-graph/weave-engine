//! Atomic logical snapshot manifests. Logical IDs are distinct from integrity digests.
use super::*;
impl Engine {
    pub(crate) fn commit_batch(
        &self,
        batch: &str,
        commits: &[SnapshotCommit],
        host: &HostContext,
    ) -> Result<(Option<String>, Vec<CommitReceipt>)> {
        if !valid_id(&host.principal)
            || batch.is_empty()
            || batch.len() > 64
            || !batch
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || commits.is_empty()
            || commits.len() > 100
        {
            return Err(err(
                "E_BATCH",
                "batch requires 1..100 snapshots and an ASCII batch identifier",
            ));
        }
        let mut graphs = HashSet::new();
        let mut schemas: BTreeMap<(String, String), GraphSchema> = BTreeMap::new();
        let mut unchanged = HashSet::new();
        let mut manifest = SnapshotManifest {
            batch_id: batch.into(),
            members: Vec::new(),
        };
        let mut existing = 0;
        json_size(&commits, 16 * 1024 * 1024)?;
        for commit in commits {
            identity_acceptance::require_external_graph(&commit.graph_id)?;
            identity_acceptance::require_external_schema(&commit.data)?;
            if !host.writable_graphs.contains(&commit.graph_id) {
                return Err(err(
                    "E_FORBIDDEN",
                    "host has not granted graph write authority",
                ));
            }
            if !valid_id(&commit.graph_id)
                || !valid_id(&commit.branch_id)
                || !graphs.insert(&commit.graph_id)
            {
                return Err(err("E_BATCH", "batch graph IDs must be valid and unique"));
            }
            validate_graph(&commit.data)?;
            if let Some(schema) = &commit.data.schema {
                let key = (schema.id.clone(), schema.revision.clone());
                if schemas.get(&key).is_some_and(|prior| prior != schema) {
                    return Err(err(
                        "E_SCHEMA_REVISION",
                        "batch schema revision binds conflicting descriptors",
                    ));
                }
                schemas.insert(key, schema.clone());
            }
            if let Some(parent) = &commit.expected_head {
                if self.load(&commit.graph_id, parent)?.as_ref() == Some(&commit.data) {
                    unchanged.insert(commit.graph_id.clone());
                }
            }

            self.validate_structures(&commit.graph_id, &commit.data)?;
            let revision = format!("logical:{batch}:{}", commit.graph_id);
            if !valid_id(&revision) {
                return Err(err("E_ID", "logical revision exceeds identifier limit"));
            }
            let digest = content_digest(
                &commit.graph_id,
                &commit.branch_id,
                &commit.expected_head,
                &commit.data,
            )?;
            if let Some(old) = self.load(&commit.graph_id, &revision)? {
                let stored: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT content_digest FROM revision_integrity WHERE revision=?1",
                        [&revision],
                        |r| r.get(0),
                    )
                    .optional()?;
                if old != commit.data || stored.as_ref() != Some(&digest) {
                    return Err(err(
                        "E_EQUIVOCATION",
                        "logical revision already binds different content",
                    ));
                }
                existing += 1;
            } else if self.head(&commit.graph_id, &commit.branch_id)? != commit.expected_head {
                return Err(err(
                    "E_CONFLICT",
                    "batch expected head differs from current branch head",
                ));
            }
            manifest.members.push(ManifestMember {
                graph_id: commit.graph_id.clone(),
                branch_id: commit.branch_id.clone(),
                revision,
                parent: commit.expected_head.clone(),
                content_digest: digest,
            });
        }
        manifest.members.sort_by(|a, b| a.graph_id.cmp(&b.graph_id));
        let manifest_id = format!(
            "manifest:{:x}",
            Sha256::digest(serde_json::to_vec(&("weave-manifest-v0.4", &manifest))?)
        );
        let stored_manifest: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM snapshot_manifests WHERE batch_id=?1",
                [batch],
                |r| r.get(0),
            )
            .optional()?;
        if existing > 0 {
            if existing != commits.len() || stored_manifest.as_ref() != Some(&manifest_id) {
                return Err(err(
                    "E_EQUIVOCATION",
                    "batch identity already binds another manifest",
                ));
            }
        } else {
            if stored_manifest.is_some() {
                return Err(err("E_EQUIVOCATION", "batch identity already allocated"));
            }
            if unchanged.len() == commits.len() {
                let receipts = commits
                    .iter()
                    .map(|c| CommitReceipt {
                        graph_id: c.graph_id.clone(),
                        branch_id: c.branch_id.clone(),
                        revision: c.expected_head.clone().expect("unchanged needs a parent"),
                        event_id: None,
                    })
                    .collect();
                return Ok((None, receipts));
            }
            self.conn.execute(
                "INSERT INTO snapshot_manifests VALUES (?1,?2,?3)",
                params![manifest_id, batch, serde_json::to_string(&manifest)?],
            )?;
            let time = self.operation_time()?;
            for member in &manifest.members {
                let data = &commits
                    .iter()
                    .find(|c| c.graph_id == member.graph_id)
                    .expect("validated manifest member")
                    .data;
                self.conn.execute(
                    "INSERT INTO revisions VALUES (?1,?2,?3,?4,?5,?6)",
                    params![
                        member.revision,
                        member.graph_id,
                        member.branch_id,
                        member.parent,
                        time,
                        serde_json::to_string(data)?
                    ],
                )?;
                self.conn.execute(
                    "INSERT INTO revision_integrity VALUES (?1,?2,?3)",
                    params![member.revision, member.content_digest, manifest_id],
                )?;
                self.conn.execute("INSERT INTO heads VALUES (?1,?2,?3) ON CONFLICT(graph_id,branch_id) DO UPDATE SET revision=excluded.revision",params![member.graph_id,member.branch_id,member.revision])?;
                if !unchanged.contains(&member.graph_id) {
                    self.conn.execute("INSERT INTO events(event_id,graph_id,branch_id,revision,actor) VALUES (?1,?2,?3,?4,?5)",params![format!("commit:{}",member.revision),member.graph_id,member.branch_id,member.revision,host.principal])?;
                }
                self.record_structures(&member.graph_id, data)?;
            }
        }
        for commit in commits {
            self.validate_required_metadata(&commit.data, host)?;
        }
        Ok((
            Some(manifest_id),
            manifest
                .members
                .iter()
                .map(|m| CommitReceipt {
                    graph_id: m.graph_id.clone(),
                    branch_id: m.branch_id.clone(),
                    revision: m.revision.clone(),
                    event_id: if unchanged.contains(&m.graph_id) {
                        None
                    } else {
                        Some(format!("commit:{}", m.revision))
                    },
                })
                .collect(),
        ))
    }
    pub fn snapshot_manifest(&self, id: &str) -> Result<Option<SnapshotManifest>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT manifest FROM snapshot_manifests WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        json.map(|j| serde_json::from_str(&j).map_err(Into::into))
            .transpose()
    }
    pub(crate) fn validate_structures(&self, graph: &str, data: &GraphData) -> Result<()> {
        if let Some(schema) = &data.schema {
            let prior: Option<String> = self
                .conn
                .query_row(
                    "SELECT descriptor FROM schema_registry WHERE id=?1 AND revision=?2",
                    params![schema.id, schema.revision],
                    |r| r.get(0),
                )
                .optional()?;
            if prior.is_some_and(|p| {
                serde_json::from_str::<GraphSchema>(&p).ok().as_ref() != Some(schema)
            }) {
                return Err(err(
                    "E_SCHEMA_REVISION",
                    "schema revision already binds a different descriptor",
                ));
            }
        }
        for (id, from, to, predicate) in data
            .edges
            .iter()
            .map(|e| (&e.id, &e.from, &e.to, &e.predicate))
            .chain(
                data.structural_edges
                    .iter()
                    .map(|e| (&e.id, &e.from, &e.to, &e.predicate)),
            )
        {
            let prior:Option<(String,String,String)>=self.conn.query_row("SELECT from_id,to_id,predicate FROM edge_structures WHERE graph_id=?1 AND edge_id=?2",params![graph,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            if prior.is_some_and(|p| p != (from.clone(), to.clone(), predicate.clone())) {
                return Err(err(
                    "E_EDGE_IDENTITY",
                    "changing edge endpoints or predicate requires a new edge identity",
                ));
            }
        }
        for assertion in &data.assertions {
            let edge_collision: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM edge_structures WHERE graph_id=?1 AND edge_id=?2)",
                params![graph, assertion.id],
                |r| r.get(0),
            )?;
            let prior:Option<(String,String)>=self.conn.query_row("SELECT edge_id,source FROM assertion_structures WHERE graph_id=?1 AND assertion_id=?2",params![graph,assertion.id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if edge_collision
                || prior.is_some_and(|p| p != (assertion.edge_id.clone(), assertion.source.clone()))
            {
                return Err(err(
                    "E_ASSERTION_IDENTITY",
                    "assertion ID cannot rebind its structural edge or source",
                ));
            }
        }
        for id in data
            .edges
            .iter()
            .map(|e| &e.id)
            .chain(data.structural_edges.iter().map(|e| &e.id))
        {
            let collision:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM assertion_structures WHERE graph_id=?1 AND assertion_id=?2)",params![graph,id],|r|r.get(0))?;
            if collision {
                return Err(err(
                    "E_ASSERTION_IDENTITY",
                    "assertion identity cannot be reused as a structural edge",
                ));
            }
        }
        Ok(())
    }
    pub(crate) fn record_structures(&self, graph: &str, data: &GraphData) -> Result<()> {
        if let Some(schema) = &data.schema {
            self.conn.execute(
                "INSERT OR IGNORE INTO schema_registry VALUES (?1,?2,?3)",
                params![schema.id, schema.revision, serde_json::to_string(schema)?],
            )?;
        }
        for (id, from, to, predicate) in data
            .edges
            .iter()
            .map(|e| (&e.id, &e.from, &e.to, &e.predicate))
            .chain(
                data.structural_edges
                    .iter()
                    .map(|e| (&e.id, &e.from, &e.to, &e.predicate)),
            )
        {
            self.conn.execute(
                "INSERT OR IGNORE INTO edge_structures VALUES (?1,?2,?3,?4,?5)",
                params![graph, id, from, to, predicate],
            )?;
        }
        for assertion in &data.assertions {
            self.conn.execute(
                "INSERT OR IGNORE INTO assertion_structures VALUES (?1,?2,?3,?4)",
                params![graph, assertion.id, assertion.edge_id, assertion.source],
            )?;
        }
        Ok(())
    }
}
pub(crate) fn content_digest(
    graph: &str,
    branch: &str,
    parent: &Option<String>,
    data: &GraphData,
) -> Result<String> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&(
            "weave-revision-v0.1",
            graph,
            branch,
            parent,
            data
        ))?)
    ))
}
impl Engine {
    pub(crate) fn query_refs(
        &self,
        data: &GraphData,
        pins: &mut BTreeMap<(String, String), Option<String>>,
    ) -> Result<(Vec<GraphRef>, bool)> {
        let mut references = refs(data);
        let mut unavailable = false;
        for attachment in &data.attachments {
            if let MetadataValue::LiveGraph {
                graph_id,
                branch_id,
            } = &attachment.value
            {
                let key = (graph_id.clone(), branch_id.clone());
                if !pins.contains_key(&key) {
                    pins.insert(key.clone(), self.head(graph_id, branch_id)?);
                }
                match &pins[&key] {
                    Some(revision) => references.push(GraphRef {
                        graph_id: graph_id.clone(),
                        revision: revision.clone(),
                    }),
                    None => unavailable = true,
                }
            }
        }
        Ok((references, unavailable))
    }
    pub(crate) fn validate_required_metadata(
        &self,
        data: &GraphData,
        host: &HostContext,
    ) -> Result<()> {
        for attachment in data.attachments.iter().filter(|a| a.required) {
            let reference = match &attachment.value {
                MetadataValue::Graph { reference } => Some(reference.clone()),
                MetadataValue::LiveGraph {
                    graph_id,
                    branch_id,
                } => self.head(graph_id, branch_id)?.map(|revision| GraphRef {
                    graph_id: graph_id.clone(),
                    revision,
                }),
                _ => continue,
            };
            let Some(reference) = reference else {
                return Err(err(
                    "E_DEPENDENCY_UNAVAILABLE",
                    "required metadata unavailable",
                ));
            };
            if !self.protected_reference_allowed(&reference.graph_id, &reference.revision, host)? {
                return Err(err(
                    "E_DEPENDENCY_UNAVAILABLE",
                    "required dependency unavailable",
                ));
            }
            let Some(target) = self.load(&reference.graph_id, &reference.revision)? else {
                return Err(err(
                    "E_DEPENDENCY_UNAVAILABLE",
                    "required metadata unavailable",
                ));
            };
            let (visible, incomplete) = self.authorized(target.clone(), host)?;
            if incomplete || visible != target {
                return Err(err(
                    "E_DEPENDENCY_UNAVAILABLE",
                    "required metadata unavailable",
                ));
            }
        }
        Ok(())
    }
}
