#!/usr/bin/env python3
"""Fail when engine code outside the SQLite fact backend names fact-store tables.

ADR-0004 section 6: only `crates/weave-engine/src/fact_store/sqlite.rs` issues
SQL against the fact log, its indexes and checkpoints. Every other module goes
through the `FactBackend` trait. This check scans the Rust sources of every
crate (`crates/*/src/**/*.rs`) for the backend's table names. It also checks
that its own table list matches the backend's `TABLES` constant, so a new
table cannot escape the check.

Code that enumerates tables dynamically (for example a retention inventory over
`sqlite_master`) must skip `fact_store::sqlite::TABLES` before issuing SQL;
that rule is reviewed in code and covered by engine tests, not by this scan.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BACKEND = ROOT / "crates" / "weave-engine" / "src" / "fact_store" / "sqlite.rs"
TABLES = ("fact_lines", "fact_revisions", "fact_log", "fact_index", "fact_checkpoints")


def declared_tables(source: str) -> tuple:
    match = re.search(r"pub const TABLES: &\[&str\] = &\[(.*?)\];", source, re.S)
    if not match:
        raise SystemExit("fact_store/sqlite.rs no longer declares `pub const TABLES`")
    return tuple(re.findall(r'"([a-z_]+)"', match.group(1)))


def main() -> int:
    backend = BACKEND.read_text(encoding="utf-8")
    if declared_tables(backend) != TABLES:
        print(f"table list drifted: backend declares {declared_tables(backend)}, check expects {TABLES}")
        return 1
    pattern = re.compile(r"\b(" + "|".join(TABLES) + r")\b")
    if not all(re.search(rf"\b{table}\b", backend.split("const SCHEMA", 1)[1]) for table in TABLES):
        print("the backend schema must define every table; the check would be vacuous")
        return 1
    violations = []
    for path in sorted((ROOT / "crates").glob("*/src/**/*.rs")):
        if path == BACKEND:
            continue
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if pattern.search(line):
                violations.append(f"{path.relative_to(ROOT)}:{number}: {line.strip()}")
    if violations:
        print("Fact-store tables referenced outside fact_store/sqlite.rs (ADR-0004 section 6):")
        print("\n".join(violations))
        return 1
    print(f"fact-store SQL boundary holds: {len(TABLES)} tables, only fact_store/sqlite.rs names them")
    return 0


if __name__ == "__main__":
    sys.exit(main())
