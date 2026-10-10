# Weave Science for Python

A dependency-free Python interface to the native Rust engine for persistent,
reproducible temporal graph and vector experiments. The native engine performs
authorization, temporal selection, query evaluation and analysis.

From the repository root:

```sh
cargo build --release -p weave-science
python3 -m venv .venv
.venv/bin/python -m pip install ./python
WEAVE_SCIENCE_BINARY="$PWD/target/release/weave-science" .venv/bin/python examples/science/temporal_vectors.py --output experiment-output
```

See [the experiment guide](../docs/PYTHON_SCIENCE.md) for the API, data import,
revision comparison, algorithms and notebook use. A Rust toolchain is needed to
build the native binary; it is not needed to run an already built binary.
