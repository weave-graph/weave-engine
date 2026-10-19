//! The generator is deterministic from its seed, on every platform: the
//! committed `fixtures/fleet-small` must match a fresh run byte for byte, and
//! the default 5,000-unit dataset has a pinned digest.
use std::collections::BTreeSet;
use std::path::Path;
use weave_hardware::generate::{generate, Config, Rng};
use weave_hardware::schema;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/fleet-small");

/// Digest of `generate(&Config::default())` (seed 42, 5,000 units, 1,000
/// updates). A change here means the generator's output changed: bump
/// `GENERATOR_VERSION` and regenerate the small fixture.
const DEFAULT_DIGEST: &str =
    "sha256:a39c56df346194581c486edf5344a1320bb8f661e54c87f2516293c66427c4f3";

fn small() -> Config {
    Config {
        seed: 7,
        units: 12,
        updates: 24,
    }
}

#[test]
fn committed_fixture_matches_a_fresh_run() {
    let files = generate(&small()).files();
    let mut on_disk = BTreeSet::new();
    for dir in [
        Path::new(FIXTURE).to_path_buf(),
        Path::new(FIXTURE).join("base"),
    ] {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let name = path
                    .strip_prefix(FIXTURE)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                on_disk.insert(name);
            }
        }
    }
    assert_eq!(on_disk, files.keys().cloned().collect::<BTreeSet<_>>());
    for (name, text) in &files {
        let committed = std::fs::read_to_string(Path::new(FIXTURE).join(name)).unwrap();
        assert!(
            committed == *text,
            "{name} differs from a fresh run; regenerate the fixture"
        );
    }
}

#[test]
fn same_seed_same_bytes_and_other_seeds_differ() {
    let a = generate(&small());
    assert_eq!(generate(&small()).files(), a.files());
    let other = generate(&Config { seed: 8, ..small() });
    assert_ne!(other.digest(), a.digest());
    let mut rng = Rng::new(1);
    let first: Vec<u64> = (0..3).map(|_| rng.next_u64()).collect();
    // SplitMix64 reference values for seed 1.
    assert_eq!(
        first,
        [0x910a2dec89025cc1, 0xbeeb8da1658eec67, 0xf893a2eefb32555e]
    );
}

#[test]
fn default_dataset_has_the_exit_scenario_size() {
    let dataset = generate(&Config::default());
    assert_eq!(dataset.base.tables["units"].len(), 5_000);
    assert_eq!(dataset.updates.len(), 1_000);
    assert_eq!(dataset.digest(), DEFAULT_DIGEST);
    // Every table is populated, and some units are customer-restricted.
    for table in schema::TABLES {
        assert!(
            !dataset.base.tables[table.name].is_empty(),
            "{} is empty",
            table.name
        );
    }
    let restricted = dataset.base.tables["units"]
        .iter()
        .filter(|u| u.contains_key("readers"))
        .count();
    assert!(restricted > 500 && restricted < 2_000, "{restricted}");
    // Updates are reported in strictly increasing order and cover every kind.
    let times: Vec<i64> = dataset
        .updates
        .iter()
        .map(|u| weave_hardware::table::parse_time(&u.reported_at).unwrap())
        .collect();
    assert!(times.windows(2).all(|w| w[0] < w[1]));
    let kinds: BTreeSet<&str> = dataset.updates.iter().map(|u| u.kind.as_str()).collect();
    assert_eq!(
        kinds,
        BTreeSet::from([
            "correction",
            "firmware_date_correction",
            "firmware_rollout",
            "incident",
            "maintenance",
            "part_replacement",
            "relocation",
        ])
    );
    // Maintenance notes have stable IDs and text, the hook for A4 embeddings.
    let notes = dataset.base.tables["maintenance"]
        .iter()
        .filter(|m| m.get("note").is_some_and(|n| n.len() > 40))
        .count();
    assert_eq!(notes, dataset.base.tables["maintenance"].len());
}
