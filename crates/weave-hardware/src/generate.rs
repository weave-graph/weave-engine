//! A deterministic, seeded synthetic fleet: a base export plus a stream of
//! incremental updates (firmware rollouts, moves, part replacements,
//! maintenance, incidents and data corrections).
//!
//! The same configuration always produces byte-identical output on every
//! platform: the generator uses its own SplitMix64 generator, integer time
//! arithmetic and ordered maps only. All names are synthetic.
use crate::schema::{self, Table};
use crate::table::{format_time, parse_time, write_csv, Batch, Row, Update};
use crate::{error, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// Version of the generator's output. Bump it when the same configuration
/// would produce different data.
pub const GENERATOR_VERSION: &str = "1";
/// Principal that every restricted record admits.
pub const ADMIN: &str = "fleet-admin";
/// A principal that only sees unrestricted records.
pub const ANALYST: &str = "fleet-analyst";

const DAY: i64 = 86_400_000;

/// Generator settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Config {
    pub seed: u64,
    pub units: usize,
    pub updates: usize,
}

impl Default for Config {
    /// The Phase A exit-scenario size: 5,000 units and 1,000 updates.
    fn default() -> Self {
        Self {
            seed: 42,
            units: 5_000,
            updates: 1_000,
        }
    }
}

/// Fleet start: the earliest commissioning date.
pub fn fleet_start() -> i64 {
    parse_time("2021-01-01").expect("constant date")
}
/// The base export describes the fleet as of this time.
pub fn base_time() -> i64 {
    parse_time("2024-07-01").expect("constant date")
}
/// Updates are reported from `base_time()` to this time.
pub fn updates_end() -> i64 {
    parse_time("2025-07-01").expect("constant date")
}

/// SplitMix64: small, fast and identical everywhere.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        ((self.next_u64() as u128 * n as u128) >> 64) as u64
    }
    /// Uniform in `lo..hi`, or `lo` when the range is empty.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        lo + self.below((hi - lo) as u64) as i64
    }
    /// True with probability `percent / 100`.
    pub fn percent(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// A generated dataset.
#[derive(Debug, Clone)]
pub struct Dataset {
    pub config: Config,
    /// The base export, as of [`base_time`].
    pub base: Batch,
    /// Updates in reporting order, one fact write each.
    pub updates: Vec<Update>,
}

#[derive(Serialize)]
struct Manifest<'a> {
    generator: &'static str,
    generator_version: &'static str,
    schema: &'static str,
    schema_revision: &'static str,
    config: &'a Config,
    base_time: String,
    updates_end: String,
    rows: BTreeMap<String, usize>,
    updates: usize,
    restricted_units: usize,
    files: BTreeMap<String, String>,
}

impl Dataset {
    /// The files of the dataset: `base/<table>.csv`, `updates.jsonl` and
    /// `manifest.json` (with the SHA-256 of every other file).
    pub fn files(&self) -> BTreeMap<String, String> {
        let mut files = BTreeMap::new();
        for table in schema::TABLES {
            let rows = self
                .base
                .tables
                .get(table.name)
                .cloned()
                .unwrap_or_default();
            files.insert(
                format!("base/{}.csv", table.name),
                write_csv(&used_columns(table, &rows), &rows),
            );
        }
        let mut updates = String::new();
        for update in &self.updates {
            updates.push_str(&update.to_line());
            updates.push('\n');
        }
        files.insert("updates.jsonl".into(), updates);
        let digests = files
            .iter()
            .map(|(name, text)| (name.clone(), sha256(text.as_bytes())))
            .collect();
        let manifest = Manifest {
            generator: "weave-hardware",
            generator_version: GENERATOR_VERSION,
            schema: schema::SCHEMA_ID,
            schema_revision: schema::SCHEMA_REVISION,
            config: &self.config,
            base_time: format_time(base_time()),
            updates_end: format_time(updates_end()),
            rows: self
                .base
                .tables
                .iter()
                .map(|(t, r)| (t.clone(), r.len()))
                .collect(),
            updates: self.updates.len(),
            restricted_units: self.base.tables["units"]
                .iter()
                .filter(|r| r.contains_key("readers"))
                .count(),
            files: digests,
        };
        let mut text = serde_json::to_string_pretty(&manifest).expect("manifest serializes");
        text.push('\n');
        files.insert("manifest.json".into(), text);
        files
    }

    /// SHA-256 of the whole dataset (of `manifest.json`, which pins every file).
    pub fn digest(&self) -> String {
        sha256(self.files()["manifest.json"].as_bytes())
    }

    /// Write the files under `dir`.
    pub fn write_to(&self, dir: &Path) -> Result<()> {
        for (name, text) in self.files() {
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| error("E_HW_OUTPUT", &format!("{}: {e}", parent.display())))?;
            }
            std::fs::write(&path, text)
                .map_err(|e| error("E_HW_OUTPUT", &format!("{}: {e}", path.display())))?;
        }
        Ok(())
    }
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

fn used_columns(table: &Table, rows: &[Row]) -> Vec<&'static str> {
    table
        .columns()
        .into_iter()
        .filter(|c| *c == table.key || rows.iter().any(|r| r.contains_key(*c)))
        .collect()
}

fn row(pairs: &[(&str, String)]) -> Row {
    pairs
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

struct Line {
    name: &'static str,
    /// Model names and their part numbers prefix.
    models: [&'static str; 2],
    /// Firmware versions with release dates.
    firmware: &'static [(&'static str, &'static str)],
}

const LINES: &[Line] = &[
    Line {
        name: "compressor",
        models: ["CX-200", "CX-300"],
        firmware: &[
            ("1.0.0", "2020-11-02"),
            ("1.1.0", "2021-06-14"),
            ("1.2.0", "2022-02-07"),
            ("2.0.0", "2022-10-17"),
            ("2.1.0", "2023-05-22"),
            ("2.2.0", "2024-01-15"),
            ("2.3.0", "2024-09-02"),
            ("3.0.0", "2025-02-10"),
        ],
    },
    Line {
        name: "pump",
        models: ["PX-40", "PX-60"],
        firmware: &[
            ("4.0.1", "2020-09-21"),
            ("4.1.0", "2021-09-06"),
            ("4.2.0", "2022-08-29"),
            ("4.2.1", "2023-03-13"),
            ("4.3.0", "2024-02-26"),
            ("4.4.0", "2024-11-11"),
            ("5.0.0", "2025-04-07"),
        ],
    },
    Line {
        name: "turbine",
        models: ["TX-9", "TX-12"],
        firmware: &[
            ("7.0", "2020-12-07"),
            ("7.1", "2021-11-15"),
            ("7.2", "2022-12-05"),
            ("7.3", "2023-09-18"),
            ("8.0", "2024-04-29"),
            ("8.1", "2024-12-09"),
            ("8.2", "2025-05-19"),
        ],
    },
];

const SLOTS: &[(&str, &str)] = &[
    ("B", "main bearing"),
    ("M", "drive motor"),
    ("C", "controller board"),
    ("S", "shaft seal"),
];
const CONFIGS: &[&str] = &["standard", "cold-climate", "high-duty"];
const REGIONS: &[(&str, &str)] = &[
    ("eu-north", "Europe/Stockholm"),
    ("eu-west", "Europe/Paris"),
    ("us-east", "America/New_York"),
    ("us-west", "America/Los_Angeles"),
    ("apac", "Asia/Singapore"),
];
const SECTORS: &[&str] = &[
    "Logistics",
    "Foods",
    "Chemicals",
    "Utilities",
    "Mining",
    "Pharma",
    "Paper",
    "Metals",
];
const SYMPTOMS: &[&str] = &[
    "vibration elevated at 2x running speed",
    "bearing temperature trending 8 degC above baseline",
    "intermittent grinding noise under load",
    "oil seepage at the shaft seal",
    "controller reported overcurrent fault E214",
    "startup takes longer than usual after cold soak",
    "broadband vibration increase after the last restart",
    "pressure ripple at low flow",
];
const ACTIONS: &[&str] = &[
    "lubricated and re-torqued the mounting bolts",
    "replaced the worn component and ran a 30 minute test",
    "realigned the coupling within 0.05 mm",
    "cleaned the cooling fins and verified airflow",
    "updated controller parameters to the site profile",
    "no fault found; monitoring the trend",
    "tightened the cable gland and resealed the housing",
];
const OUTCOMES: &[&str] = &[
    "Readings back within limits.",
    "Follow-up inspection scheduled in 30 days.",
    "Customer informed; unit returned to service.",
    "Vibration reduced by roughly half.",
    "Recommend a firmware update at the next window.",
];
const SEVERITIES: &[&str] = &["low", "medium", "high", "critical"];
const INCIDENT_TITLES: &[&str] = &[
    "Unplanned stop",
    "Overtemperature trip",
    "Abnormal vibration alarm",
    "Communication loss",
    "Seal leak reported by operator",
];

/// Mutable state while generating, holding the current row of every key.
struct State {
    rng: Rng,
    tables: BTreeMap<&'static str, BTreeMap<String, Row>>,
    units: Vec<UnitState>,
    sites_by_org: BTreeMap<String, Vec<String>>,
    sites: Vec<String>,
    technicians: Vec<String>,
    next_part: u64,
    next_work_order: u64,
    next_incident: u64,
    next_install: u64,
}

struct UnitState {
    serial: String,
    line: usize,
    model: String,
    commissioned: i64,
    decommissioned: Option<i64>,
    owner: String,
    readers: String,
    /// Current (open) rows by role.
    firmware_install: String,
    firmware_index: usize,
    placement: String,
    placements: u32,
    firmware_installs: u32,
    /// Current install key of each slot.
    parts: Vec<String>,
}

impl State {
    fn put(&mut self, table: &'static str, row: Row) -> Row {
        let key = row[schema::table(table).expect("known table").key].clone();
        self.tables
            .entry(table)
            .or_default()
            .insert(key, row.clone());
        row
    }
    fn get(&self, table: &str, key: &str) -> Row {
        self.tables[table][key].clone()
    }
    fn part_serial(&mut self) -> String {
        self.next_part += 1;
        format!("PT-{:07}", self.next_part)
    }
    fn work_order(&mut self) -> String {
        self.next_work_order += 1;
        format!("WO-{:06}", self.next_work_order)
    }
    fn install_id(&mut self) -> String {
        self.next_install += 1;
        format!("PI-{:07}", self.next_install)
    }
    fn note(&mut self, component: &str) -> String {
        let symptom = *self.rng.pick(SYMPTOMS);
        let action = *self.rng.pick(ACTIONS);
        let outcome = *self.rng.pick(OUTCOMES);
        format!("Checked the {component}: {symptom}; {action}. {outcome}")
    }
    fn technician(&mut self) -> String {
        let technicians = self.technicians.clone();
        self.rng.pick(&technicians).clone()
    }
}

fn firmware_id(line: &Line, version: &str) -> String {
    format!("{}-{version}", line.name)
}

/// Generate a dataset.
pub fn generate(config: &Config) -> Dataset {
    let mut s = State {
        rng: Rng::new(config.seed),
        tables: BTreeMap::new(),
        units: Vec::new(),
        sites_by_org: BTreeMap::new(),
        sites: Vec::new(),
        technicians: Vec::new(),
        next_part: 0,
        next_work_order: 0,
        next_incident: 0,
        next_install: 0,
    };
    base(&mut s, config);
    let base_batch = Batch {
        source: None,
        tables: s
            .tables
            .iter()
            .map(|(t, rows)| (t.to_string(), rows.values().cloned().collect()))
            .collect(),
    };
    let updates = updates(&mut s, config);
    Dataset {
        config: config.clone(),
        base: base_batch,
        updates,
    }
}

fn base(s: &mut State, config: &Config) {
    let t0 = fleet_start();
    let tb = base_time();
    // Organizations and teams.
    s.put(
        "organizations",
        row(&[
            ("org_id", "NWM".into()),
            ("name", "Northwind Machines".into()),
            ("kind", "manufacturer".into()),
        ]),
    );
    s.put(
        "organizations",
        row(&[
            ("org_id", "OPS".into()),
            ("name", "Northwind Fleet Services".into()),
            ("kind", "service".into()),
        ]),
    );
    s.put(
        "teams",
        row(&[
            ("team_id", "NWM-ENG".into()),
            ("name", "Firmware Engineering".into()),
            ("org_id", "NWM".into()),
        ]),
    );
    let field_teams = ["OPS-NORTH", "OPS-SOUTH", "OPS-EAST"];
    for team in field_teams {
        s.put(
            "teams",
            row(&[
                ("team_id", team.into()),
                ("name", format!("Field Service {}", &team[4..])),
                ("org_id", "OPS".into()),
            ]),
        );
    }
    let customers = (config.units / 250).max(4);
    let mut customer_ids = Vec::new();
    for c in 1..=customers {
        let id = format!("C{c:03}");
        let sector = SECTORS[(c - 1) % SECTORS.len()];
        let restricted = c % 5 == 3;
        let readers = if restricted {
            format!("{ADMIN};customer:{id}")
        } else {
            String::new()
        };
        s.put(
            "organizations",
            row(&[
                ("org_id", id.clone()),
                ("name", format!("Customer {c:03} {sector}")),
                ("kind", "customer".into()),
            ]),
        );
        s.put(
            "teams",
            row(&[
                ("team_id", format!("{id}-OPS")),
                ("name", format!("Customer {c:03} Operations")),
                ("org_id", id.clone()),
            ]),
        );
        s.put(
            "users",
            row(&[
                ("user_id", format!("{id}-U1")),
                ("name", format!("Operator {c:03}")),
                ("principal", format!("customer:{id}")),
                ("role", "customer-operator".into()),
                ("team_id", format!("{id}-OPS")),
                ("valid_from", format_time(t0)),
            ]),
        );
        customer_ids.push((id, readers));
    }
    s.put(
        "users",
        row(&[
            ("user_id", "ADMIN".into()),
            ("name", "Fleet Administrator".into()),
            ("principal", ADMIN.into()),
            ("role", "administrator".into()),
            ("team_id", "OPS-NORTH".into()),
        ]),
    );
    s.put(
        "users",
        row(&[
            ("user_id", "ANALYST".into()),
            ("name", "Reliability Analyst".into()),
            ("principal", ANALYST.into()),
            ("role", "analyst".into()),
            ("team_id", "NWM-ENG".into()),
        ]),
    );
    let technicians = (config.units / 25).max(3);
    for t in 1..=technicians {
        let id = format!("T{t:04}");
        let joined = t0 + s.rng.range(0, 365) * DAY;
        s.put(
            "users",
            row(&[
                ("user_id", id.clone()),
                ("name", format!("Technician {t:04}")),
                ("principal", format!("tech-{t:04}")),
                ("role", "technician".into()),
                ("team_id", field_teams[t % field_teams.len()].into()),
                ("valid_from", format_time(joined)),
            ]),
        );
        s.technicians.push(id);
    }
    // Sites, each operated by a customer.
    let sites = (config.units / 80).max(3);
    for i in 1..=sites {
        let id = format!("S{i:03}");
        let (owner, _) = &customer_ids[(i - 1) % customer_ids.len()];
        let (region, tz) = *s.rng.pick(REGIONS);
        s.put(
            "sites",
            row(&[
                ("site_id", id.clone()),
                ("name", format!("Plant {i:03}")),
                ("region", region.into()),
                ("timezone", tz.into()),
                ("org_id", owner.clone()),
            ]),
        );
        s.sites_by_org
            .entry(owner.clone())
            .or_default()
            .push(id.clone());
        s.sites.push(id);
    }
    // Models, firmware and configurations.
    for line in LINES {
        for model in line.models {
            s.put(
                "models",
                row(&[
                    ("model_id", model.into()),
                    ("name", format!("{model} {}", line.name)),
                    ("manufacturer", "Northwind Machines".into()),
                    ("product_line", line.name.into()),
                ]),
            );
            for config_name in CONFIGS {
                let id = format!("{model}-{config_name}");
                let digest = sha256(id.as_bytes());
                s.put(
                    "configurations",
                    row(&[
                        ("config_id", id.clone()),
                        ("name", format!("{model} {config_name}")),
                        ("uri", format!("git://configs/{model}/{config_name}.toml")),
                        ("digest", digest),
                    ]),
                );
            }
        }
        for (version, released) in line.firmware {
            s.put(
                "firmware",
                row(&[
                    ("firmware_id", firmware_id(line, version)),
                    ("product", line.name.into()),
                    ("version", (*version).into()),
                    ("released_at", (*released).into()),
                ]),
            );
        }
    }
    // Units with their placements, ownership, configuration, parts,
    // firmware history, sensors, series and maintenance.
    let last_commission = parse_time("2024-03-01").expect("constant date");
    for u in 0..config.units {
        let serial = format!("SN-{:06}", u + 1);
        let line_index = s.rng.below(LINES.len() as u64) as usize;
        let line = &LINES[line_index];
        let model = line.models[s.rng.below(2) as usize].to_string();
        let commissioned = s.rng.range(t0 / DAY, last_commission / DAY) * DAY;
        let decommissioned = if u > 2 && commissioned + 200 * DAY < tb && s.rng.percent(2) {
            Some(s.rng.range(commissioned / DAY + 180, tb / DAY) * DAY)
        } else {
            None
        };
        let end = decommissioned.unwrap_or(i64::MAX);
        let site = s.sites[s.rng.below(s.sites.len() as u64) as usize].clone();
        let owner = s.tables["sites"][&site]["org_id"].clone();
        let readers = customer_ids
            .iter()
            .find(|(id, _)| id == &owner)
            .map(|(_, r)| r.clone())
            .unwrap_or_default();
        let time = |t: Option<i64>| t.map(format_time).unwrap_or_default();
        s.put(
            "units",
            row(&[
                ("serial", serial.clone()),
                ("name", format!("{model}-{:06}", u + 1)),
                ("model_id", model.clone()),
                ("commissioned_at", format_time(commissioned)),
                ("decommissioned_at", time(decommissioned)),
                ("readers", readers.clone()),
            ]),
        );
        s.put(
            "ownerships",
            row(&[
                ("ownership_id", format!("OW-{serial}-1")),
                ("serial", serial.clone()),
                ("org_id", owner.clone()),
                ("valid_from", format_time(commissioned)),
                ("valid_to", time(decommissioned)),
                ("readers", readers.clone()),
            ]),
        );
        let config_name = *s.rng.pick(CONFIGS);
        s.put(
            "configuration_assignments",
            row(&[
                ("assignment_id", format!("CA-{serial}-1")),
                ("serial", serial.clone()),
                ("config_id", format!("{model}-{config_name}")),
                ("valid_from", format_time(commissioned)),
                ("valid_to", time(decommissioned)),
                ("readers", readers.clone()),
            ]),
        );
        // Placement, with an occasional move to another site of the owner.
        let mut placements = 1;
        let move_at = if s.rng.percent(8) {
            let at = s.rng.range(commissioned / DAY + 30, tb / DAY) * DAY;
            Some(at).filter(|t| *t < end)
        } else {
            None
        };
        s.put(
            "placements",
            row(&[
                ("placement_id", format!("PL-{serial}-1")),
                ("serial", serial.clone()),
                ("site_id", site.clone()),
                ("valid_from", format_time(commissioned)),
                ("valid_to", time(move_at.or(decommissioned))),
                ("readers", readers.clone()),
            ]),
        );
        if let Some(at) = move_at {
            placements = 2;
            let options = s.sites_by_org[&owner].clone();
            let next = s.rng.pick(&options).clone();
            s.put(
                "placements",
                row(&[
                    ("placement_id", format!("PL-{serial}-2")),
                    ("serial", serial.clone()),
                    ("site_id", next),
                    ("valid_from", format_time(at)),
                    ("valid_to", time(decommissioned)),
                    ("readers", readers.clone()),
                ]),
            );
        }
        // Parts: one per slot, with an occasional replacement before the base export.
        let mut parts = Vec::new();
        for (slot, description) in SLOTS {
            let part = s.part_serial();
            s.put(
                "parts",
                row(&[
                    ("part_serial", part.clone()),
                    (
                        "part_number",
                        format!("PN-{}-{slot}{}", line.name[..1].to_uppercase(), model.len()),
                    ),
                    ("description", format!("{model} {description}")),
                    ("readers", readers.clone()),
                ]),
            );
            let install = s.install_id();
            s.put(
                "part_installs",
                row(&[
                    ("install_id", install.clone()),
                    ("part_serial", part),
                    ("serial", serial.clone()),
                    ("valid_from", format_time(commissioned)),
                    ("valid_to", time(decommissioned)),
                    ("readers", readers.clone()),
                ]),
            );
            parts.push(install);
        }
        // Firmware: the newest version at commissioning, then rollouts.
        let released: Vec<i64> = line
            .firmware
            .iter()
            .map(|(_, r)| parse_time(r).expect("constant date"))
            .collect();
        let mut index = released
            .iter()
            .rposition(|r| *r <= commissioned)
            .unwrap_or(0);
        let mut from = commissioned;
        let mut installs = 0;
        let install_key = loop {
            installs += 1;
            let install_key = format!("FW-{serial}-{installs}");
            let next = (index + 1..released.len())
                .find(|i| released[*i] < tb && s.rng.percent(60))
                .map(|i| (i, (released[i] / DAY + s.rng.range(7, 90)) * DAY))
                .filter(|(_, at)| *at > from && *at < tb.min(end));
            s.put(
                "firmware_installs",
                row(&[
                    ("install_id", install_key.clone()),
                    ("serial", serial.clone()),
                    ("firmware_id", firmware_id(line, line.firmware[index].0)),
                    ("valid_from", format_time(from)),
                    ("valid_to", time(next.map(|(_, at)| at).or(decommissioned))),
                    ("readers", readers.clone()),
                ]),
            );
            match next {
                Some((i, at)) => {
                    index = i;
                    from = at;
                }
                None => break install_key,
            }
        };
        // Sensors and yearly series references.
        for (suffix, kind, unit_of_measure, rate) in [
            ("V", "vibration", "mm/s", "2560"),
            ("T", "temperature", "degC", "1"),
        ] {
            let sensor = format!("{serial}-{suffix}");
            s.put(
                "sensors",
                row(&[
                    ("sensor_id", sensor.clone()),
                    ("kind", kind.into()),
                    ("unit_of_measure", unit_of_measure.into()),
                    ("sample_rate_hz", rate.into()),
                    ("readers", readers.clone()),
                ]),
            );
            s.put(
                "sensor_mounts",
                row(&[
                    ("mount_id", format!("SM-{sensor}-1")),
                    ("sensor_id", sensor.clone()),
                    ("serial", serial.clone()),
                    ("valid_from", format_time(commissioned)),
                    ("valid_to", time(decommissioned)),
                    ("readers", readers.clone()),
                ]),
            );
            let mut year = 2021;
            loop {
                let ys = parse_time(&format!("{year}-01-01")).expect("constant date");
                let ye = parse_time(&format!("{}-01-01", year + 1)).expect("constant date");
                let (start, stop) = (ys.max(commissioned), ye.min(tb).min(end));
                if ys >= tb.min(end) {
                    break;
                }
                if start < stop {
                    let id = format!("{sensor}-{year}");
                    s.put(
                        "series",
                        row(&[
                            ("series_ref_id", id.clone()),
                            ("sensor_id", sensor.clone()),
                            (
                                "store_uri",
                                format!("parquet://fleet-telemetry/{kind}/year={year}"),
                            ),
                            ("series_id", sensor.clone()),
                            ("start", format_time(start)),
                            ("end", format_time(stop)),
                            (
                                "digest",
                                sha256(format!("{}|{id}|{start}|{stop}", config.seed).as_bytes()),
                            ),
                            ("readers", readers.clone()),
                        ]),
                    );
                }
                year += 1;
            }
        }
        let mut unit = UnitState {
            serial: serial.clone(),
            line: line_index,
            model,
            commissioned,
            decommissioned,
            owner,
            readers,
            firmware_install: install_key,
            firmware_index: index,
            placement: format!("PL-{serial}-{placements}"),
            placements,
            firmware_installs: installs,
            parts,
        };
        // Maintenance history. Unit 1 always has a part replacement and unit
        // 2 an incident, so every type and relation occurs in any fleet.
        let visits = s.rng.below(3) as i64;
        for _ in 0..visits {
            let at = s.rng.range(commissioned / DAY + 10, tb.min(end) / DAY) * DAY + 9 * 3_600_000;
            if at < tb.min(end) {
                maintenance(s, &unit, at, "inspection", None);
            }
        }
        if u == 0 || (decommissioned.is_none() && s.rng.percent(10)) {
            let at = s.rng.range(commissioned / DAY + 60, tb / DAY) * DAY + 13 * 3_600_000;
            if at < end {
                replace_part(s, &mut unit, at);
            }
        }
        if u == 1 || s.rng.percent(4) {
            let at = s.rng.range(commissioned / DAY + 30, tb.min(end) / DAY) * DAY + 3 * 3_600_000;
            let closed = at + s.rng.range(1, 72) * 3_600_000;
            incident(s, &unit, at, Some(closed));
        }
        s.units.push(unit);
    }
}

/// A maintenance row; `part` names the replaced component.
fn maintenance(s: &mut State, unit: &UnitState, at: i64, kind: &str, part: Option<&str>) -> Row {
    let work_order = s.work_order();
    let component = match part {
        Some(p) => p.to_string(),
        None => s.rng.pick(SLOTS).1.to_string(),
    };
    let note = s.note(&component);
    let technician = s.technician();
    let hours = s.rng.range(1, 6);
    s.put(
        "maintenance",
        row(&[
            ("work_order", work_order),
            ("kind", kind.into()),
            ("serial", unit.serial.clone()),
            ("performed_by", technician),
            ("started_at", format_time(at)),
            ("ended_at", format_time(at + hours * 3_600_000)),
            ("note", note),
            ("readers", unit.readers.clone()),
        ]),
    )
}

/// Replace a random part at `at`: close its install, add a new part and an
/// install that replaces it, and a replacement work order. Returns the rows.
fn replace_part(s: &mut State, unit: &mut UnitState, at: i64) -> Vec<(&'static str, Row)> {
    let slot = s.rng.below(SLOTS.len() as u64) as usize;
    let mut old = s.get("part_installs", &unit.parts[slot]);
    if parse_time(&old["valid_from"]).expect("generated time") >= at {
        return vec![];
    }
    old.insert("valid_to".into(), format_time(at));
    let old_part = old["part_serial"].clone();
    let old_meta = s.get("parts", &old_part);
    let part = s.part_serial();
    let new_part = row(&[
        ("part_serial", part.clone()),
        ("part_number", old_meta["part_number"].clone()),
        ("description", old_meta["description"].clone()),
        ("readers", unit.readers.clone()),
    ]);
    let wo = maintenance(s, unit, at, "replacement", Some(SLOTS[slot].1));
    let install = s.install_id();
    let new_install = row(&[
        ("install_id", install.clone()),
        ("part_serial", part),
        ("serial", unit.serial.clone()),
        ("valid_from", format_time(at)),
        (
            "valid_to",
            unit.decommissioned.map(format_time).unwrap_or_default(),
        ),
        ("replaces", old_part),
        ("work_order", wo["work_order"].clone()),
        ("readers", unit.readers.clone()),
    ]);
    unit.parts[slot] = install;
    vec![
        ("part_installs", s.put("part_installs", old)),
        ("parts", s.put("parts", new_part)),
        ("part_installs", s.put("part_installs", new_install)),
        ("maintenance", wo),
    ]
}

fn incident(s: &mut State, unit: &UnitState, at: i64, closed: Option<i64>) -> Row {
    s.next_incident += 1;
    let id = format!("INC-{:05}", s.next_incident);
    let severity = *s.rng.pick(SEVERITIES);
    let title = *s.rng.pick(INCIDENT_TITLES);
    let symptom = *s.rng.pick(SYMPTOMS);
    s.put(
        "incidents",
        row(&[
            ("incident_id", id),
            ("severity", severity.into()),
            ("title", title.into()),
            ("serial", unit.serial.clone()),
            ("opened_at", format_time(at)),
            ("closed_at", closed.map(format_time).unwrap_or_default()),
            (
                "description",
                format!("{title} on {}: {symptom}.", unit.serial),
            ),
            ("readers", unit.readers.clone()),
        ]),
    )
}

fn updates(s: &mut State, config: &Config) -> Vec<Update> {
    let (tb, te) = (base_time(), updates_end());
    let span = (te - tb) / (config.updates.max(1) as i64);
    let mut out = Vec::new();
    let mut seq = 0;
    while out.len() < config.updates {
        seq += 1;
        // Strictly increasing report times, at most one per minute slot.
        let reported =
            tb + (out.len() as i64) * span + 60_000 + s.rng.range(0, (span - 120_000).max(1));
        let roll = s.rng.below(100);
        let Some((kind, source, rows)) = update(s, roll, reported) else {
            continue;
        };
        let mut tables: BTreeMap<String, Vec<Row>> = BTreeMap::new();
        for (table, row) in rows {
            tables.entry(table.to_string()).or_default().push(row);
        }
        out.push(Update {
            seq: out.len() as u64 + 1,
            kind: kind.into(),
            reported_at: format_time(reported),
            batch: Batch {
                source: Some(source.into()),
                tables,
            },
        });
        if seq > config.updates * 20 + 100 {
            break;
        }
    }
    out
}

type Rows = Vec<(&'static str, Row)>;

/// One update at report time `reported`, or `None` when the drawn kind does
/// not apply to the drawn unit (the caller draws again).
fn update(s: &mut State, roll: u64, reported: i64) -> Option<(&'static str, &'static str, Rows)> {
    let index = s.rng.below(s.units.len() as u64) as usize;
    let unit = &s.units[index];
    if unit.decommissioned.is_some() || unit.commissioned >= reported - 30 * DAY {
        return None;
    }
    // The world change happened up to ten days before it was reported.
    let happened = reported - s.rng.range(0, 10 * DAY / 60_000) * 60_000;
    match roll {
        0..=44 => rollout(s, index, happened),
        45..=54 => correct_rollout_date(s, index, reported),
        55..=62 => relocate(s, index, happened),
        63..=74 => {
            let mut unit = std::mem::replace(&mut s.units[index], placeholder());
            let rows = replace_part(s, &mut unit, happened);
            s.units[index] = unit;
            (!rows.is_empty()).then_some(("part_replacement", "field-service", rows))
        }
        75..=86 => {
            let unit = std::mem::replace(&mut s.units[index], placeholder());
            let row = maintenance(s, &unit, happened, "inspection", None);
            s.units[index] = unit;
            Some(("maintenance", "field-service", vec![("maintenance", row)]))
        }
        87..=91 => {
            let unit = std::mem::replace(&mut s.units[index], placeholder());
            let row = incident(s, &unit, happened, None);
            s.units[index] = unit;
            Some(("incident", "field-service", vec![("incidents", row)]))
        }
        _ => fix_record(s, index),
    }
}

fn placeholder() -> UnitState {
    UnitState {
        serial: String::new(),
        line: 0,
        model: String::new(),
        commissioned: 0,
        decommissioned: None,
        owner: String::new(),
        readers: String::new(),
        firmware_install: String::new(),
        firmware_index: 0,
        placement: String::new(),
        placements: 0,
        firmware_installs: 0,
        parts: vec![],
    }
}

/// Roll out the next released firmware: close the current install and open
/// a new one (both valid-time facts).
fn rollout(
    s: &mut State,
    index: usize,
    happened: i64,
) -> Option<(&'static str, &'static str, Rows)> {
    let unit = &s.units[index];
    let line = &LINES[unit.line];
    let next = unit.firmware_index + 1;
    let released = parse_time(line.firmware.get(next)?.1).expect("constant date");
    if released > happened {
        return None;
    }
    let mut current = s.get("firmware_installs", &unit.firmware_install);
    if parse_time(&current["valid_from"]).expect("generated time") >= happened {
        return None;
    }
    current.insert("valid_to".into(), format_time(happened));
    let serial = unit.serial.clone();
    let readers = unit.readers.clone();
    let installs = unit.firmware_installs + 1;
    let key = format!("FW-{serial}-{installs}");
    let new = row(&[
        ("install_id", key.clone()),
        ("serial", serial),
        ("firmware_id", firmware_id(line, line.firmware[next].0)),
        ("valid_from", format_time(happened)),
        ("readers", readers),
    ]);
    let unit = &mut s.units[index];
    unit.firmware_install = key;
    unit.firmware_index = next;
    unit.firmware_installs = installs;
    Some((
        "firmware_rollout",
        "fleet-ops",
        vec![
            ("firmware_installs", s.put("firmware_installs", current)),
            ("firmware_installs", s.put("firmware_installs", new)),
        ],
    ))
}

/// The installer logged the last rollout on the wrong day: move the boundary
/// between the last two installs (a correction of both valid times).
fn correct_rollout_date(
    s: &mut State,
    index: usize,
    reported: i64,
) -> Option<(&'static str, &'static str, Rows)> {
    let unit = &s.units[index];
    if unit.firmware_installs < 2 {
        return None;
    }
    let serial = unit.serial.clone();
    let last_key = unit.firmware_install.clone();
    let prev_key = format!("FW-{serial}-{}", unit.firmware_installs - 1);
    let mut last = s.get("firmware_installs", &last_key);
    let mut prev = s.get("firmware_installs", &prev_key);
    let boundary = parse_time(&last["valid_from"]).expect("generated time");
    let lower = parse_time(&prev["valid_from"]).expect("generated time");
    let shift = s.rng.range(1, 6) * DAY * if s.rng.percent(50) { 1 } else { -1 };
    let moved = boundary + shift;
    if moved <= lower || moved >= reported {
        return None;
    }
    last.insert("valid_from".into(), format_time(moved));
    prev.insert("valid_to".into(), format_time(moved));
    Some((
        "firmware_date_correction",
        "data-steward",
        vec![
            ("firmware_installs", s.put("firmware_installs", prev)),
            ("firmware_installs", s.put("firmware_installs", last)),
        ],
    ))
}

/// Move a unit to another site of its owner.
fn relocate(
    s: &mut State,
    index: usize,
    happened: i64,
) -> Option<(&'static str, &'static str, Rows)> {
    let unit = &s.units[index];
    let options = s.sites_by_org.get(&unit.owner)?.clone();
    let mut current = s.get("placements", &unit.placement);
    let site = s.rng.pick(&options).clone();
    if site == current["site_id"]
        || parse_time(&current["valid_from"]).expect("generated time") >= happened
    {
        return None;
    }
    current.insert("valid_to".into(), format_time(happened));
    let unit = &mut s.units[index];
    unit.placements += 1;
    let key = format!("PL-{}-{}", unit.serial, unit.placements);
    unit.placement = key.clone();
    let new = row(&[
        ("placement_id", key),
        ("serial", unit.serial.clone()),
        ("site_id", site),
        ("valid_from", format_time(happened)),
        ("readers", unit.readers.clone()),
    ]);
    Some((
        "relocation",
        "erp",
        vec![
            ("placements", s.put("placements", current)),
            ("placements", s.put("placements", new)),
        ],
    ))
}

/// A data steward fixes a wrong attribute: a unit name, a part description
/// or a sensor's sample rate.
fn fix_record(s: &mut State, index: usize) -> Option<(&'static str, &'static str, Rows)> {
    let unit = &s.units[index];
    let serial = unit.serial.clone();
    let model = unit.model.clone();
    let slot = s.rng.below(SLOTS.len() as u64) as usize;
    let install = unit.parts[slot].clone();
    match s.rng.below(3) {
        0 => {
            let mut row = s.get("units", &serial);
            let fixed = format!("{model}-{} rev{}", &serial[3..], s.rng.range(2, 9));
            if row.get("name") == Some(&fixed) {
                return None;
            }
            row.insert("name".into(), fixed);
            Some((
                "correction",
                "data-steward",
                vec![("units", s.put("units", row))],
            ))
        }
        1 => {
            let part = s.get("part_installs", &install)["part_serial"].clone();
            let mut row = s.get("parts", &part);
            let fixed = format!("{} (rev {})", SLOTS[slot].1, s.rng.range(2, 9));
            if row.get("description") == Some(&fixed) {
                return None;
            }
            row.insert("description".into(), fixed);
            Some((
                "correction",
                "data-steward",
                vec![("parts", s.put("parts", row))],
            ))
        }
        _ => {
            let mut row = s.get("sensors", &format!("{serial}-V"));
            let rate = *s.rng.pick(&["2048", "3200", "5120"]);
            if row["sample_rate_hz"] == rate {
                return None;
            }
            row.insert("sample_rate_hz".into(), rate.into());
            Some((
                "correction",
                "data-steward",
                vec![("sensors", s.put("sensors", row))],
            ))
        }
    }
}
