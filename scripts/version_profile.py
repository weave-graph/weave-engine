"""Read the current native storage marker for version-fence acceptance checks."""
from pathlib import Path
import re


def store_marker():
    source = (Path(__file__).resolve().parents[1] / "crates/weave-engine/src/lib.rs").read_text()
    return int(re.search(r"pub const STORAGE_VERSION: i64 = (\d+);", source).group(1))
