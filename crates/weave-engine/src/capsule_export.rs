//! Authenticated whole-capsule read endpoint. Receipt is never installation authority.
use super::*;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use weave_policy::{Action, AdmissionProof, Operation, Request};

const REQUEST: &str = "weave-capsule-export-request-0.1";
const RESPONSE: &str = "weave-capsule-export-response-0.1";
const STORED: &str = "weave-stored-capsule-export-0.1";
const DOMAIN: &str = "weave-capsule-export-response-signature-v0.1";
const SMALL: usize = 16 * 1024;
const CAPSULE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapsuleExportRequest {
    pub format: String,
    pub root: GraphRef,
    pub branch_id: String,
    pub server_key: String,
    pub response_audience: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapsuleExportBinding {
    pub format: String,
    pub server_key: String,
    pub server_audience: String,
    pub recipient_subject: String,
    pub recipient_audience: String,
    pub request_nonce: String,
    pub request_body_digest: String,
    pub policy_epoch: String,
    pub root: GraphRef,
    pub branch_id: String,
    pub contract_version: String,
    pub capsule_format: String,
    pub capsule_digest: String,
    pub served_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SignedCapsuleExport {
    pub binding: CapsuleExportBinding,
    #[serde(deserialize_with = "bounded_capsule")]
    pub capsule: Capsule,
    pub signature: String,
}
/// Out-of-band serving key and allowed subject/recipient endpoint pairs. Never JSON authority.
pub struct CapsuleExportSigner {
    key: SigningKey,
    audience: String,
    pairings: BTreeMap<String, BTreeSet<String>>,
}
impl CapsuleExportSigner {
    pub fn new(
        key: SigningKey,
        audience: String,
        pairings: BTreeMap<String, BTreeSet<String>>,
    ) -> Result<Self> {
        if !valid_id(&audience) || pairings.len() > 1000 {
            return Err(binding_error());
        }
        json_size(&pairings, 1024 * 1024)?;
        for (subject, endpoints) in &pairings {
            public_key(subject)?;
            if endpoints.is_empty()
                || endpoints.len() > 1000
                || endpoints.iter().any(|s| !valid_id(s))
            {
                return Err(binding_error());
            }
        }
        Ok(Self {
            key,
            audience,
            pairings,
        })
    }
}
/// An explicitly paired peer and currently outstanding request, created by trusted requester code.
pub struct CapsuleExportExpectation {
    paired_key: String,
    server_audience: String,
    subject: String,
    recipient_audience: String,
    request: CapsuleExportRequest,
    outstanding: Request,
}
impl CapsuleExportExpectation {
    pub fn new(
        paired_key: String,
        server_audience: String,
        recipient_subject: String,
        recipient_audience: String,
        request: CapsuleExportRequest,
        outstanding: Request,
    ) -> Result<Self> {
        let body = request_bytes(&request)?;
        public_key(&paired_key)?;
        public_key(&recipient_subject)?;
        if !valid_id(&server_audience)
            || !valid_id(&recipient_audience)
            || paired_key != request.server_key
            || recipient_audience != request.response_audience
            || outstanding.version != weave_policy::REQUEST_VERSION
            || outstanding.subject != recipient_subject
            || outstanding.audience != server_audience
            || outstanding.body_digest != weave_policy::body_digest(&body)
            || outstanding.operation != operation(&request)
            || outstanding.issued_at_ms < 0
            || outstanding.expires_at_ms <= outstanding.issued_at_ms
        {
            return Err(binding_error());
        }
        decode::<32>(&outstanding.nonce)?;
        json_size(&outstanding, SMALL)?;
        Ok(Self {
            paired_key,
            server_audience,
            subject: recipient_subject,
            recipient_audience,
            request,
            outstanding,
        })
    }
}
/// Authenticated bytes only. Receiving or accepting them still requires independent authority.
#[derive(Debug)]
pub struct VerifiedCapsuleExport(Capsule);
impl VerifiedCapsuleExport {
    pub fn capsule(&self) -> &Capsule {
        &self.0
    }
    pub fn into_capsule(self) -> Capsule {
        self.0
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredExport {
    format: String,
    response: SignedCapsuleExport,
    #[serde(deserialize_with = "bounded_list")]
    dependencies: Vec<GraphRef>,
}
fn bounded_list<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Vec<T>, D::Error> {
    struct List<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for List<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "at most 1000 export records")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Vec<T>, A::Error> {
            let mut out = Vec::new();
            while out.len() < 1000 {
                match seq.next_element()? {
                    Some(value) => out.push(value),
                    None => return Ok(out),
                }
            }
            // IgnoredAny avoids allocating a 1001st nested record before rejecting it.
            if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom("export record limit"));
            }
            Ok(out)
        }
    }
    d.deserialize_seq(List(std::marker::PhantomData))
}
fn bounded_capsule<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Capsule, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Wire {
        format: String,
        root: GraphRef,
        #[serde(deserialize_with = "bounded_list")]
        revisions: Vec<CapsuleRevision>,
        #[serde(deserialize_with = "bounded_list")]
        external_dependencies: Vec<GraphRef>,
        #[serde(default, deserialize_with = "bounded_list")]
        manifests: Vec<SnapshotManifest>,
    }
    let w = Wire::deserialize(d)?;
    Ok(Capsule {
        format: w.format,
        root: w.root,
        revisions: w.revisions,
        external_dependencies: w.external_dependencies,
        manifests: w.manifests,
    })
}
fn binding_error() -> Error {
    err("E_EXPORT_BINDING", "capsule export binding invalid")
}
fn unavailable() -> Error {
    err("E_UNAVAILABLE", "capsule export unavailable")
}
fn unavailable_graph(error: Error) -> Error {
    if matches!(
        error.code.as_str(),
        "E_SCOPE"
            | "E_UNAVAILABLE"
            | "E_DEPENDENCY_UNAVAILABLE"
            | "E_CAPSULE_VERSION"
            | "E_IDENTITY_RESERVED"
            | "E_GOV_RESERVED"
            | "E_RESERVED_NAMESPACE"
    ) {
        unavailable()
    } else {
        error
    }
}
fn operation(request: &CapsuleExportRequest) -> Operation {
    Operation {
        action: Action::Read,
        graph_id: request.root.graph_id.clone(),
        branch_id: request.branch_id.clone(),
    }
}
fn request_bytes(request: &CapsuleExportRequest) -> Result<Vec<u8>> {
    json_size(request, SMALL)?;
    if request.format != REQUEST
        || !valid_id(&request.root.graph_id)
        || !valid_id(&request.root.revision)
        || !valid_id(&request.branch_id)
        || !valid_id(&request.response_audience)
    {
        return Err(binding_error());
    }
    public_key(&request.server_key)?;
    Ok(serde_json::to_vec(request)?)
}
fn decode<const N: usize>(value: &str) -> Result<[u8; N]> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(binding_error());
    }
    let mut result = [0; N];
    for (i, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let digit = |b| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
        result[i] = digit(pair[0]) * 16 + digit(pair[1]);
    }
    Ok(result)
}
fn public_key(value: &str) -> Result<VerifyingKey> {
    let bytes = decode::<32>(value.strip_prefix("ed25519:").ok_or_else(binding_error)?)?;
    let key = VerifyingKey::from_bytes(&bytes).map_err(|_| binding_error())?;
    if key.is_weak() {
        return Err(binding_error());
    }
    Ok(key)
}
fn signature_bytes(binding: &CapsuleExportBinding) -> Result<Vec<u8>> {
    json_size(binding, SMALL)?;
    Ok(serde_json::to_vec(&(DOMAIN, binding))?)
}
fn validate_response(response: &SignedCapsuleExport, key: &str) -> Result<()> {
    json_size(response, CAPSULE + SMALL + 1024)?;
    let b = &response.binding;
    if b.format != RESPONSE
        || b.server_key != key
        || b.root != response.capsule.root
        || ![VERSION, "0.18.0", "0.17.0", "0.16.0"].contains(&b.contract_version.as_str())
        || (b.contract_version == "0.16.0"
            && (response.capsule.format == "weave-capsule-0.3"
                || response
                    .capsule
                    .revisions
                    .iter()
                    .any(|r| influence::requires_v017(&r.data))))
        || b.capsule_format != response.capsule.format
        || b.served_at_ms < 0
    {
        return Err(binding_error());
    }
    json_size(&response.capsule, CAPSULE)?;
    if b.capsule_digest != weave_policy::body_digest(&serde_json::to_vec(&response.capsule)?) {
        return Err(binding_error());
    }
    public_key(key)?
        .verify_strict(
            &signature_bytes(b)?,
            &Signature::from_bytes(&decode::<64>(&response.signature)?),
        )
        .map_err(|_| err("E_SIGNATURE", "capsule export signature invalid"))?;
    Ok(())
}
fn matching_response(
    response: &SignedCapsuleExport,
    request: &CapsuleExportRequest,
    outstanding: &Request,
) -> Result<()> {
    let b = &response.binding;
    if b.root != request.root
        || b.branch_id != request.branch_id
        || b.server_key != request.server_key
        || b.server_audience != outstanding.audience
        || b.recipient_subject != outstanding.subject
        || b.recipient_audience != request.response_audience
        || b.request_nonce != outstanding.nonce
        || b.request_body_digest != outstanding.body_digest
    {
        return Err(binding_error());
    }
    Ok(())
}

/// Borrowed streaming traversal: charge repeated dependency occurrences before retaining pins.
pub(crate) fn visit_dependencies(
    record: &CapsuleRevision,
    work: &mut usize,
    mut visit: impl FnMut(&str, &str) -> Result<()>,
) -> Result<()> {
    let mut one = |graph: &str, revision: &str| {
        charge_work(work)?;
        if !valid_id(graph) || !valid_id(revision) {
            return Err(binding_error());
        }
        visit(graph, revision)
    };
    macro_rules! pins {
        ($values:expr) => {
            for r in $values {
                one(&r.graph_id, &r.revision)?;
            }
        };
    }
    let data = &record.data;
    if let Some(parent) = &record.parent {
        one(&record.graph_id, parent)?;
    }
    if let Some(t) = &data.context_typing {
        pins!(t.selected.iter());
        for w in &t.witnesses {
            one(&w.context.graph_id, &w.context.revision)?;
            one(&w.definition.graph_id, &w.definition.revision)?;
            pins!(&w.anchor_nodes);
        }
    }
    if let Some(i) = &data.influence {
        pins!(&i.assertions);
        pins!(&i.nodes);
        pins!(&i.snapshots);
    }
    for n in &data.nodes {
        pins!(&n.metadata);
        pins!(&n.derived_from);
        pins!(&n.derived_nodes);
        pins!(&n.derived_snapshots);
        if let Some(r) = n
            .context_scope
            .as_ref()
            .and_then(ContextSelection::reference)
        {
            one(&r.graph_id, &r.revision)?;
        }
    }
    for e in &data.edges {
        pins!(&e.metadata);
        pins!(&e.derived_from);
        pins!(&e.derived_nodes);
        pins!(&e.derived_snapshots);
        pins!(e.structural_ref.iter());
        pins!(e.assertion_context.iter());
    }
    for e in &data.structural_edges {
        pins!(&e.metadata);
    }
    for a in &data.assertions {
        pins!(&a.metadata);
        pins!(a.context.iter());
        pins!(&a.derived_from);
        pins!(&a.derived_nodes);
        pins!(&a.derived_snapshots);
    }
    for g in data
        .edges
        .iter()
        .flat_map(|e| &e.derivations)
        .chain(data.assertions.iter().flat_map(|a| &a.derivations))
    {
        pins!(&g.premises);
        pins!(&g.node_premises);
        pins!(&g.input_snapshots);
    }
    for a in &data.attachments {
        pins!(&a.derived_from);
        pins!(&a.derived_nodes);
        pins!(&a.derived_snapshots);
        pins!(a.context.iter());
        pins!(a.origin.iter());
        match &a.value {
            MetadataValue::Graph { reference } => one(&reference.graph_id, &reference.revision)?,
            MetadataValue::LiveGraph { .. } => return Err(unavailable()),
            _ => {}
        }
    }
    Ok(())
}
fn charge_work(work: &mut usize) -> Result<()> {
    *work += 1;
    if *work > 10000 {
        return Err(err("E_BUDGET", "export traversal work exceeded"));
    }
    Ok(())
}

/// Reconstruct a complete closure from authenticated capsule bytes, never from a receipt's metadata.
fn complete_closure(capsule: &Capsule, work: &mut usize) -> Result<Vec<GraphRef>> {
    json_size(capsule, CAPSULE)?;
    if !capsule.external_dependencies.is_empty() {
        return Err(unavailable());
    }
    if capsule.format != "weave-capsule-0.3"
        && capsule
            .revisions
            .iter()
            .any(|r| influence::requires_v017(&r.data))
    {
        return Err(binding_error());
    }
    if capsule.revisions.len() > 1000 || capsule.manifests.len() > 1000 {
        return Err(err("E_BUDGET", "export closure exceeds budget"));
    }
    if ![
        "weave-capsule-0.1",
        "weave-capsule-0.2",
        "weave-capsule-0.3",
    ]
    .contains(&capsule.format.as_str())
        || (capsule.format == "weave-capsule-0.1" && !capsule.manifests.is_empty())
    {
        return Err(binding_error());
    }
    let mut records = BTreeMap::new();
    for record in &capsule.revisions {
        charge_work(work)?;
        identity_acceptance::require_external_graph(&record.graph_id).map_err(unavailable_graph)?;
        identity_acceptance::require_external_schema(&record.data).map_err(unavailable_graph)?;
        if !valid_id(&record.graph_id)
            || !valid_id(&record.revision)
            || !valid_id(&record.branch_id)
            || record
                .parent
                .as_ref()
                .is_some_and(|p| !valid_id(p) || p == &record.revision)
            || record
                .data
                .attachments
                .iter()
                .any(|a| matches!(a.value, MetadataValue::LiveGraph { .. }))
            || records
                .insert((record.graph_id.clone(), record.revision.clone()), record)
                .is_some()
        {
            return Err(unavailable());
        }
        validate_graph(&record.data)?;
        if !record.revision.starts_with("logical:") && record.digest()? != record.revision {
            return Err(err("E_INTEGRITY", "export revision content mismatch"));
        }
    }
    let mut manifest_members = BTreeMap::new();
    let mut batches = HashSet::new();
    for manifest in &capsule.manifests {
        if manifest.members.len() > 100 || !batches.insert(&manifest.batch_id) {
            return Err(binding_error());
        }
        // Reject repeated membership before hashing a manifest's record bodies again.
        for member in &manifest.members {
            charge_work(work)?;
            let key = (member.graph_id.clone(), member.revision.clone());
            if !records.contains_key(&key) || manifest_members.insert(key, manifest).is_some() {
                return Err(binding_error());
            }
        }
        capsule::verify_manifest(manifest, &capsule.revisions)?;
    }
    for (key, record) in &records {
        if record.revision.starts_with("logical:") && !manifest_members.contains_key(key) {
            return Err(binding_error());
        }
    }
    let root = (capsule.root.graph_id.clone(), capsule.root.revision.clone());
    let mut queued = BTreeSet::from([root.clone()]);
    let mut pending = VecDeque::from([(root, 0)]);
    while let Some((key, depth)) = pending.pop_front() {
        if depth > 32 {
            return Err(err("E_BUDGET", "export closure depth exceeded"));
        }
        let record = records.get(&key).ok_or_else(unavailable)?;
        let mut enqueue = |graph: &str, revision: &str| -> Result<()> {
            let key = (graph.to_owned(), revision.to_owned());
            if !records.contains_key(&key) {
                return Err(unavailable());
            }
            if !queued.contains(&key) {
                if queued.len() >= 1000 {
                    return Err(err("E_BUDGET", "export closure exceeds budget"));
                }
                queued.insert(key.clone());
                pending.push_back((key, depth + 1));
            }
            Ok(())
        };
        visit_dependencies(record, work, &mut enqueue)?;
        if let Some(manifest) = manifest_members.get(&key) {
            for member in &manifest.members {
                charge_work(work)?;
                enqueue(&member.graph_id, &member.revision)?;
            }
        }
    }
    if queued.len() != records.len() {
        return Err(binding_error());
    }
    // Parent cycles are not metadata cycles. Bound independent ancestry validation.
    for record in records.values() {
        let mut current = *record;
        let mut seen = HashSet::new();
        while let Some(parent) = &current.parent {
            charge_work(work)?;
            if !seen.insert(&current.revision) {
                return Err(binding_error());
            }
            current = records
                .get(&(current.graph_id.clone(), parent.clone()))
                .ok_or_else(unavailable)?;
        }
    }
    Ok(queued
        .into_iter()
        .map(|(graph_id, revision)| GraphRef { graph_id, revision })
        .collect())
}

impl Engine {
    pub fn admit_capsule_export(
        &mut self,
        proof: &AdmissionProof,
        request: &CapsuleExportRequest,
        signer: &CapsuleExportSigner,
    ) -> Result<Admitted<SignedCapsuleExport>> {
        self.admit_capsule_export_boundary(proof, request, signer, || {})
    }
    #[cfg(feature = "recovery-testing")]
    pub fn admit_capsule_export_test_before_commit(
        &mut self,
        proof: &AdmissionProof,
        request: &CapsuleExportRequest,
        signer: &CapsuleExportSigner,
        before_commit: impl FnOnce(),
    ) -> Result<Admitted<SignedCapsuleExport>> {
        self.admit_capsule_export_boundary(proof, request, signer, before_commit)
    }
    fn admit_capsule_export_boundary(
        &mut self,
        proof: &AdmissionProof,
        request: &CapsuleExportRequest,
        signer: &CapsuleExportSigner,
        before_commit: impl FnOnce(),
    ) -> Result<Admitted<SignedCapsuleExport>> {
        let _read_scope = self.read_budget.enter();
        let body = request_bytes(request)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let _clock = self.operation_write_scope()?;
        let verified = self.verify_admission(proof, &body, &operation(request))?;
        if request.server_key != weave_policy::public_key(&signer.key)
            || signer.audience != proof.request.request.audience
            || !signer
                .pairings
                .get(verified.principal())
                .is_some_and(|pairs| pairs.contains(&request.response_audience))
        {
            return Err(binding_error());
        }
        let scopes = &proof
            .chain
            .last()
            .expect("verified chain")
            .capability
            .scopes;
        admission::require_scope(
            scopes,
            &request.root.graph_id,
            &request.branch_id,
            &[Action::Read, Action::Traverse],
        )?;
        let host = HostContext::new(verified.principal(), []);
        let mut steps = 0;
        if !self.reachable_revision_bounded(
            &request.root.graph_id,
            &request.branch_id,
            &request.root.revision,
            &mut steps,
        )? {
            return Err(unavailable());
        }
        let prior: Option<StoredExport> = self.prior_admission(&verified)?;
        let duplicate = prior.is_some();
        let mut receipt = if let Some(receipt) = prior {
            if receipt.format != STORED
                || receipt.response.binding.policy_epoch != verified.policy_epoch()
            {
                return Err(binding_error());
            }
            validate_response(&receipt.response, &request.server_key)?;
            matching_response(&receipt.response, request, &proof.request.request)?;
            receipt
        } else {
            let capsule = self
                .export_capsule_profile(&request.root, &host, true, &mut steps)
                .map_err(unavailable_graph)?;
            let dependencies = Vec::new();
            let binding = CapsuleExportBinding {
                format: RESPONSE.into(),
                server_key: request.server_key.clone(),
                server_audience: signer.audience.clone(),
                recipient_subject: verified.principal().into(),
                recipient_audience: request.response_audience.clone(),
                request_nonce: proof.request.request.nonce.clone(),
                request_body_digest: verified.body_digest().into(),
                policy_epoch: verified.policy_epoch().into(),
                root: request.root.clone(),
                branch_id: request.branch_id.clone(),
                contract_version: VERSION.into(),
                capsule_format: capsule.format.clone(),
                capsule_digest: weave_policy::body_digest(&serde_json::to_vec(&capsule)?),
                served_at_ms: self.operation_time()?,
            };
            let signature = signer
                .key
                .sign(&signature_bytes(&binding)?)
                .to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            StoredExport {
                format: STORED.into(),
                response: SignedCapsuleExport {
                    binding,
                    capsule,
                    signature,
                },
                dependencies,
            }
        };
        let exact = complete_closure(&receipt.response.capsule, &mut steps)?;
        if !duplicate {
            receipt.dependencies = exact.clone();
        }
        if exact != receipt.dependencies
            || receipt.response.binding.served_at_ms > self.operation_time()?
        {
            return Err(binding_error());
        }
        for record in &receipt.response.capsule.revisions {
            let reference = GraphRef {
                graph_id: record.graph_id.clone(),
                revision: record.revision.clone(),
            };
            self.require_reference_scope(&reference, scopes, &mut steps)
                .map_err(unavailable_graph)?;
            if !self.protected_reference_allowed(&record.graph_id, &record.revision, &host)? {
                return Err(unavailable());
            }
            let stored = self.revision_record(&reference)?.ok_or_else(unavailable)?;
            if &stored != record {
                return Err(err(
                    "E_INTEGRITY",
                    "cached export differs from stored revision",
                ));
            }
            let (visible, incomplete) = self.authorized(stored.data, &host)?;
            if !whole_graph_visible(&record.data, visible, incomplete) {
                return Err(unavailable());
            }
            self.validate_required_metadata(&record.data, &host)
                .map_err(unavailable_graph)?;
        }
        json_size(&receipt.response, CAPSULE + SMALL + 1024)?;
        if !duplicate {
            self.record_admission(&verified, &receipt)?;
        }
        before_commit();
        tx.commit()?;
        Ok(Admitted {
            duplicate,
            result: receipt.response,
        })
    }
    pub fn verify_capsule_export_response(
        &self,
        response: &SignedCapsuleExport,
        expectation: &CapsuleExportExpectation,
    ) -> Result<VerifiedCapsuleExport> {
        let _tx = self.optional_read_transaction()?;
        let _clock = self.operation_scope()?;
        let now = self.operation_time()?;
        if now < expectation.outstanding.issued_at_ms
            || now >= expectation.outstanding.expires_at_ms
        {
            return Err(err("E_EXPIRED", "capsule export expectation expired"));
        }
        validate_response(response, &expectation.paired_key)?;
        matching_response(response, &expectation.request, &expectation.outstanding)?;
        let b = &response.binding;
        if b.server_audience != expectation.server_audience
            || b.recipient_subject != expectation.subject
            || b.recipient_audience != expectation.recipient_audience
            || b.served_at_ms > now
        {
            return Err(binding_error());
        }
        complete_closure(&response.capsule, &mut 0)?;
        Ok(VerifiedCapsuleExport(response.capsule.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vector() -> (SigningKey, SignedCapsuleExport) {
        let key = SigningKey::from_bytes(&[7; 32]);
        let capsule = Capsule {
            format: "weave-capsule-0.1".into(),
            root: GraphRef {
                graph_id: "g".into(),
                revision: "r".into(),
            },
            revisions: vec![],
            external_dependencies: vec![],
            manifests: vec![],
        };
        let binding = CapsuleExportBinding {
            format: RESPONSE.into(),
            server_key: weave_policy::public_key(&key),
            server_audience: "server".into(),
            recipient_subject: weave_policy::public_key(&SigningKey::from_bytes(&[8; 32])),
            recipient_audience: "receiver".into(),
            request_nonce: "09".repeat(32),
            request_body_digest: weave_policy::body_digest(b"request"),
            policy_epoch: "epoch1".into(),
            root: capsule.root.clone(),
            branch_id: "main".into(),
            contract_version: "0.16.0".into(),
            capsule_format: capsule.format.clone(),
            capsule_digest: weave_policy::body_digest(&serde_json::to_vec(&capsule).unwrap()),
            served_at_ms: 20,
        };
        let signature = key
            .sign(&signature_bytes(&binding).unwrap())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        (
            key,
            SignedCapsuleExport {
                binding,
                capsule,
                signature,
            },
        )
    }
    #[test]
    fn fixed_response_vector_and_cross_domain_rejection() {
        let (key, mut response) = vector();
        assert_eq!(response.signature, "28132a0e0a34c6e4197ad6430e456a298d1485fe3f72c4f722fdde2e05e5fa438e56dd412a3bce5f24e69641c48a99f9324affb565a11f06d932905022279105");
        validate_response(&response, &weave_policy::public_key(&key)).unwrap();
        let wrong =
            serde_json::to_vec(&("weave-request-signature-v0.1", &response.binding)).unwrap();
        response.signature = key
            .sign(&wrong)
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            validate_response(&response, &weave_policy::public_key(&key))
                .unwrap_err()
                .code,
            "E_SIGNATURE"
        );
        assert!(public_key(&format!("ed25519:{}", "00".repeat(32))).is_err());
    }
    #[test]
    fn repeated_dependency_occurrences_are_charged_before_enqueue() {
        let mut node: Node =
            serde_json::from_value(serde_json::json!({"id":"n","entity_id":"n","space_id":"s"}))
                .unwrap();
        node.metadata = vec![
            GraphRef {
                graph_id: "g".into(),
                revision: "r".into()
            };
            10001
        ];
        let record = CapsuleRevision {
            graph_id: "g".into(),
            branch_id: "main".into(),
            revision: "r".into(),
            parent: None,
            data: GraphData {
                nodes: vec![node],
                ..Default::default()
            },
        };
        let mut count = 0;
        let error = visit_dependencies(&record, &mut 0, |_, _| {
            count += 1;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, "E_BUDGET");
        assert_eq!(count, 10000);
    }
}
