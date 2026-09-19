# Contract 0.13.0: pinned native graph services

Two additive graph expressions expose existing native APIs under the runtime's trusted HostContext:

- `{"kind":"resolve_identity","selection":IdentityResolve}` selects an exact accepted mapping/policy revision, exact source NodeRef, target space, valid instant and explicit context. DTO fields are `mapping_id`, `revision`, `policy:{id,revision}`, `source:{graph_id,revision,node_id}`, `target_space`, `valid_at`, `context`.
- `{"kind":"cluster","selection":ClusterRequest}` selects an exact source GraphRef, context, valid instant, predicate and bounded lazy level count. Fields are `source:{graph_id,revision}`, `context`, `valid_at`, `predicate`, `levels` (0–10000, with tighter effective source/proof budgets).

These selectors cannot install a policy, approve a candidate, inject an authorized Snapshot or mutate graph storage. Every read rechecks current source and policy authority. Identity output remains scoped partial adjacency with original IDs and exact accepted/source proofs. Cluster output remains partial navigation with isolated-node and selected-assertion influences; it never prunes an exact query. Missing required native state or authority is an explicit error.

Both expressions return ordinary reusable graph values. Source frontend bindings can feed their results into pure graph functions and subsequent algebra. The first surface requires explicit literal pins; discovering a current mapping or silently reading a live graph head is not implied. Context is explicitly Default or a pinned GraphRef. Clocked native views replace only the valid instant; all pins remain fixed. Cached data and change records revalidate current authority before disclosure.

Earlier protocols reject these operators recursively before any writes. Existing protocol 0.1–0.12 features retain their previous meaning. The shared host_types module contains selection DTOs only; native policy installation and acceptance stay outside this portable wire contract.

Evidence: native_services and identity_acceptance tests compare expression outputs with direct APIs, check no new events, reject nested old-wire operators atomically, preserve revocation behavior and exercise explicit view ticks. These operators do not complete authenticated peer sync, governance or clustering quality requirements.
