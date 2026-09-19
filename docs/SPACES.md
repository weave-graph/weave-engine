# Explicit geometry foundation

`weave-spaces` is a portable pure calculation crate. It implements part of the
white papers' physical/vector geometry requirements, not the complete identity,
clustering, learned mapping, spatial indexing or adapter service.

The host must obtain every sample, descriptor and transform from authorized pinned
graph evidence and supply its effective restrictions. These functions do not
authenticate a principal string, attest a caller-supplied source revision, install
spaces, or grant publication rights. No signed network endpoint exposes them yet.
Treat this API like the pure contract evaluator, not like an admission boundary.
An adapter integrating it must re-resolve evidence and check current capabilities
before publishing through the engine; graph IDs alone are not authentication.

## Operations

- `distance` requires identical full space descriptors, including space revision,
  frame and units or encoder, preprocessing, dimension and metric. Physical values
  are positions in a three-dimensional frame. Direction annotations cannot be used
  as points. Embedding cosine distance is undefined on a zero vector. Equal vector
  length never establishes compatibility.
- `transform` requires an explicit directed, revisioned rigid transform with an
  orthonormal right-handed rotation (tolerance 1e-10). Source and target descriptors
  must match exactly. Metres, centimetres and millimetres convert explicitly;
  translation is expressed in target units. Direction annotations rotate and
  scale without translation. This has no effect on semantic edge direction.
  `Direction` here denotes a unit-bearing displacement/directional annotation;
  normalized dimensionless orientation vectors need a distinct future type.
- `project_axes` creates a display-only `NavigationProjection`, with original space,
  source revision evidence, projection revision and algorithm `axis-selection/1`.
  It is marked approximate and cannot be passed to authoritative distance APIs.
  It is a simple inspectable projection, not a learned reduction or nearest-neighbor
  recall guarantee.

All inputs require pinned source references and half-open valid time containing
the requested timestamp. Results carry every influencing source, the intersection
of valid times and the intersection of reader sets. `Visibility::Public` is
explicit; an empty `Principals` set denies everybody. No embedding or coordinate
is considered an anonymized release. Missing geometry needs no synthetic vector.

Input/output work is bounded by 4096 vector dimensions, 256 source references and
1 MiB serialized evidence. Size is counted with a streaming writer. Unavailable
readers are checked before data-dependent diagnostics; invalid dimensions,
non-finite coordinates, numerical overflow and unsupported domains fail explicitly.
Floating calculations are tolerance-tested, not promised bitwise identical on
every CPU or suitable for content addressing without a numerical protocol.
Transforms conservatively reject intermediate floating-point overflow even when
later unit scaling could have brought an exact mathematical result into range.

## Evidence and remaining integration

`cargo test -p weave-spaces` exercises descriptor/revision mismatches, orthogonal
and very large embedding vectors, zero vectors, restricted source intersection,
half-open validity, explicit unit/frame transformation, direction independence,
display projection lineage, malformed numbers and bounded failure. The crate
also builds for `wasm32-unknown-unknown`.

Identity equivalence, semantic relationships and learned mappings are not inferred
from transforms. Counterpart discovery, historical identity decisions, governed
space installation, authorized graph-to-sample extraction, engine adapter scheduling,
language syntax, uncertainty models and vector indexing remain open requirements.
