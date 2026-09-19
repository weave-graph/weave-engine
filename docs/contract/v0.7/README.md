# Contract 0.7.0: finite graph rules

Version 0.7 adds `GraphExpression::Reason { input, rules }`. Existing 0.1–0.6 program profiles remain explicit and accepted; any nested Reason under an older version rejects. The graph input is an authorized materialized value. The result retains input facts and adds the bounded fixed-point closure as another composable graph value. Reason is pure: no automatic persistence, external effects or clock reads.

`RuleSet { id, revision, rules }` contains 1–64 named rules. Each rule has one binary head atom, 1–8 binary body atoms and an explicit `allow_cross_space` flag. Atoms bind exact existing graph-local node IDs through variables or existing-node constants and specify a predicate and polarity. Every head variable must occur in a body atom. No terms, nodes, arithmetic values, identity mappings or new spaces are generated. Crossing spaces requires the rule's explicit declaration.

Positive and negative atoms match their respective asserted evidence. Negative evidence is not absence. There is no negation-as-failure, closed-world assumption, aggregate, default rule, winner selection or confidence arithmetic. Matching contextual claims reject with `E_CONTEXT_REQUIRED` until context selection is implemented. Authorization is inherited from the input; a rule cannot install capabilities or roots.

## Fixed point, time and proof

Rule joins intersect half-open valid intervals exactly; disjoint intervals never derive a conclusion. A fact key includes predicate, exact endpoint IDs, polarity and interval. Each alternative support group is keyed by its canonical source assertion leaf set and context status. Cyclic rule paths cannot manufacture endlessly nested proof identities: traces are finite records keyed by rule, bindings, body facts and conclusion. Repeated saturation preserves the existing result rather than accumulating duplicate source corroboration.

Every derived edge retains OR alternatives with AND premises, exact source graph/revision/assertion leaves, rule module ID/revision/content digest, bindings and flattened derivation trace. Typed input receives a synthesized closed schema whose identity includes the resulting descriptor. Source manifest label collisions reject instead of assigning two digests to the same module revision. Source digests identify computation inputs; they are not proof of author identity.

The finite domain consists of existing nodes, predicates appearing in input/rules, source interval endpoints and finite source support subsets. Saturation therefore has a finite mathematical state space. The implementation additionally enforces host limits: 100,000 evaluation steps, 128 rounds, 10,000 fact keys, 128 support alternatives per fact and the existing 32 MiB serialized materialization ceiling. Limits are host-owned, not JSON authority fields. `E_RULE_BUDGET` means no result was produced; it never masquerades as a complete empty graph. A failing operation rolls back all earlier commands in its program. Partial input coverage remains partial even after saturation.

## Views and evidence

Named views accept Reason and clock their input queries with explicit ticks. Full recomputation remains the reference oracle: source changes or expiry retract derived membership. No incremental-performance claim follows from this implementation.

Portable tests exercise cycles/idempotence, proof alternatives, signed negative evidence, time/disjointness, schema closure, unsafe modules/constants, contextual rejection, source-label conflicts and resource failures. Native tests in `crates/weave-engine/tests/rules.rs` additionally verify version guards, whole-program rollback, persistence of private source restrictions and tick/restart retraction against a fresh evaluation. The contract builds for WASM; SQLite storage remains native.

General Datalog, richer patterns, aggregates, contextual reasoning, trusted closed-world completeness and distributed rule execution remain required follow-on work where specified in the original papers; this finite binary profile is not a claim of full language conformance.
