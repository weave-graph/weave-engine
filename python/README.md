# Weave Science for Python

`weave_science` connects scripts and notebooks to the native Weave engine for
persistent temporal graph and vector experiments. Rust performs query evaluation,
authorization and analysis; Python has no required runtime dependencies.

From the repository root, with a Rust/native compiler toolchain and Python 3.10+:

```sh
cargo build --release --locked -p weave-science
python3 -m venv .venv
.venv/bin/python -m pip install ./python
export WEAVE_SCIENCE_BINARY="$PWD/target/release/weave-science"
.venv/bin/python examples/science/temporal_vectors.py --output experiment-output
```

On Windows PowerShell:

```powershell
cargo build --release --locked -p weave-science
py -3 -m venv .venv
.venv\Scripts\python.exe -m pip install ./python
$env:WEAVE_SCIENCE_BINARY = "$PWD\target\release\weave-science.exe"
.venv\Scripts\python.exe examples/science/temporal_vectors.py --output experiment-output
```

With a prebuilt binary, omit Cargo and point `WEAVE_SCIENCE_BINARY` at the
executable. Use a fresh example output directory on each run. After installation,
this Python snippet creates its own temporary database:

```python
import tempfile
from pathlib import Path
from weave_science import Engine, node, edge

work = Path(tempfile.mkdtemp(prefix="weave-demo-"))
engine = Engine(work / "demo.sqlite", actor="scientist", write_graphs=["demo"])
receipt = engine.import_graph("demo", nodes=[node("a"), node("b")],
                               edges=[edge("ab", "a", "b")])
selected = engine.query("demo", revision=receipt.revision)
components = engine.analyze(selected, algorithm="components", mode="weak", valid_at=0)
assert components.analysis["components"] == [["a", "b"]]
components.save_experiment(work / "trial.json")
print(work)
```

The [experiment guide](../docs/PYTHON_SCIENCE.md) contains a complete temporal
correction/replay experiment, exact vector search, table/CSV import, notebook
setup and API/error reference. Read [Science concepts](../docs/SCIENCE_CONCEPTS.md)
for identities, valid versus recorded time, CAS and authorization.

Imports replace a full graph snapshot and require `expected_head` for updates.
Results preserve exact inputs, provenance and coverage; artifact replay rechecks
current authorization and rejects changed live dependencies. Optional pandas and
NetworkX conversion is available through the `tables` and `networkx` extras.
