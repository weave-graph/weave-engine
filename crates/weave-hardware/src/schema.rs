//! The hardware reference model: node types, relations, and the tables that
//! map tabular exports onto them. `docs/HARDWARE_MODEL.md` documents it; the
//! graph schema returned by [`graph_schema`] enforces it in the engine.
use std::collections::BTreeMap;
use weave_contract::{EdgeSchema, GraphSchema, NodeSchema, PropertySchema, ScalarType};

/// Schema identifier stored in the graph header.
pub const SCHEMA_ID: &str = "weave.hardware";
/// Schema and mapping revision. A change to how rows map to facts bumps it.
pub const SCHEMA_REVISION: &str = "1";
/// Space of every node of the model.
pub const SPACE: &str = "hw";
/// Importer name recorded in claimed provenance (`method.importer`).
pub const IMPORTER: &str = "weave-hardware";
/// Attachment key of a series reference on a sensor.
pub const SERIES_KEY: &str = "hw:series";
/// Attachment key of maintenance-note and incident text.
pub const NOTE_KEY: &str = "hw:note";

/// Value type of a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    Str,
    Int,
    Float,
    Bool,
    /// Unix-epoch milliseconds, or a UTC date or date-time in the CSV.
    Time,
}

impl Ty {
    fn scalar(self) -> ScalarType {
        match self {
            Ty::Str => ScalarType::String,
            Ty::Int | Ty::Time => ScalarType::Integer,
            Ty::Float => ScalarType::Float,
            Ty::Bool => ScalarType::Boolean,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Ty::Str => "string",
            Ty::Int => "integer",
            Ty::Float => "float",
            Ty::Bool => "boolean",
            Ty::Time => "time",
        }
    }
}

/// A node property read from a column of the same name.
#[derive(Debug, Clone, Copy)]
pub struct Prop {
    pub column: &'static str,
    pub ty: Ty,
    pub required: bool,
}

const fn req(column: &'static str, ty: Ty) -> Prop {
    Prop {
        column,
        ty,
        required: true,
    }
}
const fn opt(column: &'static str, ty: Ty) -> Prop {
    Prop {
        column,
        ty,
        required: false,
    }
}

/// A node type of the model.
#[derive(Debug, Clone, Copy)]
pub struct NodeType {
    /// Schema type name, also the node's `type_id` (for example `hw.Unit`).
    pub type_id: &'static str,
    /// Prefix of node IDs (`unit:SN-000001`).
    pub prefix: &'static str,
    pub props: &'static [Prop],
    pub doc: &'static str,
}

/// A relation (edge type) of the model.
#[derive(Debug, Clone, Copy)]
pub struct Relation {
    /// Edge predicate, also the edge's `type_id` (for example `hw:runs`).
    pub predicate: &'static str,
    pub from: &'static str,
    pub to: &'static str,
    pub props: &'static [Prop],
    pub doc: &'static str,
}

pub const ORGANIZATION: NodeType = NodeType {
    type_id: "hw.Organization",
    prefix: "org",
    props: &[req("name", Ty::Str), req("kind", Ty::Str)],
    doc: "A customer, operator, manufacturer or internal organization",
};
pub const TEAM: NodeType = NodeType {
    type_id: "hw.Team",
    prefix: "team",
    props: &[req("name", Ty::Str)],
    doc: "A team inside an organization",
};
pub const USER: NodeType = NodeType {
    type_id: "hw.User",
    prefix: "user",
    props: &[
        req("name", Ty::Str),
        opt("principal", Ty::Str),
        opt("role", Ty::Str),
    ],
    doc: "A person; `principal` is the Weave principal used in `readers`",
};
pub const SITE: NodeType = NodeType {
    type_id: "hw.Site",
    prefix: "site",
    props: &[
        req("name", Ty::Str),
        opt("region", Ty::Str),
        opt("timezone", Ty::Str),
    ],
    doc: "A physical location where units operate",
};
pub const MODEL: NodeType = NodeType {
    type_id: "hw.Model",
    prefix: "model",
    props: &[
        req("name", Ty::Str),
        opt("manufacturer", Ty::Str),
        opt("product_line", Ty::Str),
    ],
    doc: "A product model (a unit's design)",
};
pub const FIRMWARE: NodeType = NodeType {
    type_id: "hw.FirmwareVersion",
    prefix: "fw",
    props: &[
        req("product", Ty::Str),
        req("version", Ty::Str),
        opt("released_at", Ty::Time),
    ],
    doc: "A firmware or software version",
};
pub const CONFIGURATION: NodeType = NodeType {
    type_id: "hw.Configuration",
    prefix: "config",
    props: &[
        req("name", Ty::Str),
        opt("uri", Ty::Str),
        opt("digest", Ty::Str),
    ],
    doc: "A named configuration (a parameter set stored outside the graph)",
};
pub const UNIT: NodeType = NodeType {
    type_id: "hw.Unit",
    prefix: "unit",
    props: &[req("serial", Ty::Str), opt("name", Ty::Str)],
    doc: "A physical asset, identified by its serial number. Node valid time is commissioning to decommissioning",
};
pub const PART: NodeType = NodeType {
    type_id: "hw.Part",
    prefix: "part",
    props: &[
        req("part_serial", Ty::Str),
        req("part_number", Ty::Str),
        opt("description", Ty::Str),
    ],
    doc: "A serialized physical part (bearing, board, motor)",
};
pub const SENSOR: NodeType = NodeType {
    type_id: "hw.Sensor",
    prefix: "sensor",
    props: &[
        req("kind", Ty::Str),
        opt("unit_of_measure", Ty::Str),
        opt("sample_rate_hz", Ty::Float),
    ],
    doc: "A sensor. Its samples stay in a series store; `hw:series` attachments reference them",
};
pub const MAINTENANCE: NodeType = NodeType {
    type_id: "hw.MaintenanceEvent",
    prefix: "maint",
    props: &[req("kind", Ty::Str)],
    doc: "A maintenance work order. Node valid time is when the work happened; the note is an `hw:note` attachment",
};
pub const INCIDENT: NodeType = NodeType {
    type_id: "hw.Incident",
    prefix: "incident",
    props: &[req("severity", Ty::Str), req("title", Ty::Str)],
    doc:
        "An incident. Node valid time is open to close; the description is an `hw:note` attachment",
};

/// Every node type.
pub const NODE_TYPES: &[NodeType] = &[
    ORGANIZATION,
    TEAM,
    USER,
    SITE,
    MODEL,
    FIRMWARE,
    CONFIGURATION,
    UNIT,
    PART,
    SENSOR,
    MAINTENANCE,
    INCIDENT,
];

const WORK_ORDER: &[Prop] = &[opt("work_order", Ty::Str)];

/// Every relation.
pub const RELATIONS: &[Relation] = &[
    Relation {
        predicate: "hw:part_of",
        from: TEAM.type_id,
        to: ORGANIZATION.type_id,
        props: &[],
        doc: "A team belongs to an organization",
    },
    Relation {
        predicate: "hw:member_of",
        from: USER.type_id,
        to: TEAM.type_id,
        props: &[],
        doc: "A user is a member of a team",
    },
    Relation {
        predicate: "hw:operated_by",
        from: SITE.type_id,
        to: ORGANIZATION.type_id,
        props: &[],
        doc: "An organization operates a site",
    },
    Relation {
        predicate: "hw:has_model",
        from: UNIT.type_id,
        to: MODEL.type_id,
        props: &[],
        doc: "A unit is an instance of a model",
    },
    Relation {
        predicate: "hw:located_at",
        from: UNIT.type_id,
        to: SITE.type_id,
        props: &[],
        doc: "A unit is at a site during the edge's valid time",
    },
    Relation {
        predicate: "hw:owned_by",
        from: UNIT.type_id,
        to: ORGANIZATION.type_id,
        props: &[],
        doc: "An organization owns a unit during the edge's valid time",
    },
    Relation {
        predicate: "hw:installed_in",
        from: PART.type_id,
        to: UNIT.type_id,
        props: WORK_ORDER,
        doc: "A part is installed in a unit during the edge's valid time",
    },
    Relation {
        predicate: "hw:replaced",
        from: PART.type_id,
        to: PART.type_id,
        props: WORK_ORDER,
        doc: "A part replaced another part, from the replacement time on",
    },
    Relation {
        predicate: "hw:runs",
        from: UNIT.type_id,
        to: FIRMWARE.type_id,
        props: &[],
        doc: "A unit runs a firmware version during the edge's valid time",
    },
    Relation {
        predicate: "hw:configured_with",
        from: UNIT.type_id,
        to: CONFIGURATION.type_id,
        props: &[],
        doc: "A unit uses a configuration during the edge's valid time",
    },
    Relation {
        predicate: "hw:measured_by",
        from: UNIT.type_id,
        to: SENSOR.type_id,
        props: &[],
        doc: "A sensor is mounted on and measures a unit during the edge's valid time",
    },
    Relation {
        predicate: "hw:performed_on",
        from: MAINTENANCE.type_id,
        to: UNIT.type_id,
        props: &[],
        doc: "Maintenance was performed on a unit",
    },
    Relation {
        predicate: "hw:performed_by",
        from: MAINTENANCE.type_id,
        to: USER.type_id,
        props: &[],
        doc: "A user performed maintenance",
    },
    Relation {
        predicate: "hw:affected",
        from: INCIDENT.type_id,
        to: UNIT.type_id,
        props: &[],
        doc: "An incident affected a unit",
    },
];

/// Look up a relation by predicate.
pub fn relation(predicate: &str) -> &'static Relation {
    RELATIONS
        .iter()
        .find(|r| r.predicate == predicate)
        .expect("relation is part of the model")
}

/// Where an edge endpoint comes from.
#[derive(Debug, Clone, Copy)]
pub enum End {
    /// The node this row defines.
    Row,
    /// The node of this type whose key is in the named column.
    Column(&'static str, NodeType),
}

/// One edge a row produces.
#[derive(Debug, Clone, Copy)]
pub struct EdgeMap {
    pub predicate: &'static str,
    /// Prefix of the edge ID; the row key follows it.
    pub id_prefix: &'static str,
    pub from: End,
    pub to: End,
    /// Column holding the valid-time start. `None`, or an empty value when
    /// `start_required` is false, means time 0.
    pub start: Option<&'static str>,
    pub start_required: bool,
    /// Column holding the exclusive valid-time end. Empty means open.
    pub end: Option<&'static str>,
    /// When set, the edge exists only if this column is nonempty.
    pub when: Option<&'static str>,
}

/// The node a row defines.
#[derive(Debug, Clone, Copy)]
pub struct NodeMap {
    pub node: NodeType,
    /// Node valid time `[start, end)` columns, when the node has one.
    pub valid: Option<(&'static str, &'static str)>,
}

/// One input table.
#[derive(Debug, Clone, Copy)]
pub struct Table {
    pub name: &'static str,
    /// Column holding the row's stable key. Fact IDs derive from it, which
    /// makes it the idempotency key of every fact the row produces.
    pub key: &'static str,
    pub node: Option<NodeMap>,
    pub edges: &'static [EdgeMap],
    /// Column whose text becomes an `hw:note` attachment on the row's node.
    pub note: Option<&'static str>,
    /// True for the series table: each row is one `hw:series` attachment.
    pub series: bool,
    /// Claimed `source` when neither the row nor the import names one.
    pub default_source: &'static str,
    /// Further columns the table accepts (for example foreign keys).
    pub extra_columns: &'static [&'static str],
}

const fn edge(
    predicate: &'static str,
    id_prefix: &'static str,
    from: End,
    to: End,
    start: Option<&'static str>,
    end: Option<&'static str>,
) -> EdgeMap {
    EdgeMap {
        predicate,
        id_prefix,
        from,
        to,
        start,
        start_required: false,
        end,
        when: None,
    }
}
const fn interval_edge(
    predicate: &'static str,
    id_prefix: &'static str,
    from: End,
    to: End,
) -> EdgeMap {
    EdgeMap {
        predicate,
        id_prefix,
        from,
        to,
        start: Some("valid_from"),
        start_required: true,
        end: Some("valid_to"),
        when: None,
    }
}

const UNIT_COL: End = End::Column("serial", UNIT);

/// Every input table, in dependency order (a table only references nodes
/// defined by earlier tables).
pub const TABLES: &[Table] = &[
    Table {
        name: "organizations",
        key: "org_id",
        node: Some(NodeMap {
            node: ORGANIZATION,
            valid: None,
        }),
        edges: &[],
        note: None,
        series: false,
        default_source: "directory",
        extra_columns: &[],
    },
    Table {
        name: "teams",
        key: "team_id",
        node: Some(NodeMap {
            node: TEAM,
            valid: None,
        }),
        edges: &[edge(
            "hw:part_of",
            "part_of",
            End::Row,
            End::Column("org_id", ORGANIZATION),
            None,
            None,
        )],
        note: None,
        series: false,
        default_source: "directory",
        extra_columns: &["org_id"],
    },
    Table {
        name: "users",
        key: "user_id",
        node: Some(NodeMap {
            node: USER,
            valid: None,
        }),
        edges: &[EdgeMap {
            when: Some("team_id"),
            ..edge(
                "hw:member_of",
                "member_of",
                End::Row,
                End::Column("team_id", TEAM),
                Some("valid_from"),
                Some("valid_to"),
            )
        }],
        note: None,
        series: false,
        default_source: "directory",
        extra_columns: &["team_id", "valid_from", "valid_to"],
    },
    Table {
        name: "sites",
        key: "site_id",
        node: Some(NodeMap {
            node: SITE,
            valid: None,
        }),
        edges: &[EdgeMap {
            when: Some("org_id"),
            ..edge(
                "hw:operated_by",
                "operated_by",
                End::Row,
                End::Column("org_id", ORGANIZATION),
                None,
                None,
            )
        }],
        note: None,
        series: false,
        default_source: "erp",
        extra_columns: &["org_id"],
    },
    Table {
        name: "models",
        key: "model_id",
        node: Some(NodeMap {
            node: MODEL,
            valid: None,
        }),
        edges: &[],
        note: None,
        series: false,
        default_source: "plm",
        extra_columns: &[],
    },
    Table {
        name: "firmware",
        key: "firmware_id",
        node: Some(NodeMap {
            node: FIRMWARE,
            valid: None,
        }),
        edges: &[],
        note: None,
        series: false,
        default_source: "plm",
        extra_columns: &[],
    },
    Table {
        name: "configurations",
        key: "config_id",
        node: Some(NodeMap {
            node: CONFIGURATION,
            valid: None,
        }),
        edges: &[],
        note: None,
        series: false,
        default_source: "plm",
        extra_columns: &[],
    },
    Table {
        name: "units",
        key: "serial",
        node: Some(NodeMap {
            node: UNIT,
            valid: Some(("commissioned_at", "decommissioned_at")),
        }),
        edges: &[EdgeMap {
            start_required: true,
            ..edge(
                "hw:has_model",
                "has_model",
                End::Row,
                End::Column("model_id", MODEL),
                Some("commissioned_at"),
                None,
            )
        }],
        note: None,
        series: false,
        default_source: "erp",
        extra_columns: &["model_id", "commissioned_at", "decommissioned_at"],
    },
    Table {
        name: "placements",
        key: "placement_id",
        node: None,
        edges: &[interval_edge(
            "hw:located_at",
            "located_at",
            UNIT_COL,
            End::Column("site_id", SITE),
        )],
        note: None,
        series: false,
        default_source: "erp",
        extra_columns: &["serial", "site_id", "valid_from", "valid_to"],
    },
    Table {
        name: "ownerships",
        key: "ownership_id",
        node: None,
        edges: &[interval_edge(
            "hw:owned_by",
            "owned_by",
            UNIT_COL,
            End::Column("org_id", ORGANIZATION),
        )],
        note: None,
        series: false,
        default_source: "erp",
        extra_columns: &["serial", "org_id", "valid_from", "valid_to"],
    },
    Table {
        name: "configuration_assignments",
        key: "assignment_id",
        node: None,
        edges: &[interval_edge(
            "hw:configured_with",
            "configured_with",
            UNIT_COL,
            End::Column("config_id", CONFIGURATION),
        )],
        note: None,
        series: false,
        default_source: "fleet-ops",
        extra_columns: &["serial", "config_id", "valid_from", "valid_to"],
    },
    Table {
        name: "parts",
        key: "part_serial",
        node: Some(NodeMap {
            node: PART,
            valid: None,
        }),
        edges: &[],
        note: None,
        series: false,
        default_source: "plm",
        extra_columns: &[],
    },
    Table {
        name: "part_installs",
        key: "install_id",
        node: None,
        edges: &[
            interval_edge(
                "hw:installed_in",
                "installed_in",
                End::Column("part_serial", PART),
                UNIT_COL,
            ),
            EdgeMap {
                predicate: "hw:replaced",
                id_prefix: "replaced",
                from: End::Column("part_serial", PART),
                to: End::Column("replaces", PART),
                start: Some("valid_from"),
                start_required: true,
                end: None,
                when: Some("replaces"),
            },
        ],
        note: None,
        series: false,
        default_source: "erp",
        extra_columns: &[
            "part_serial",
            "serial",
            "valid_from",
            "valid_to",
            "replaces",
            "work_order",
        ],
    },
    Table {
        name: "firmware_installs",
        key: "install_id",
        node: None,
        edges: &[interval_edge(
            "hw:runs",
            "runs",
            UNIT_COL,
            End::Column("firmware_id", FIRMWARE),
        )],
        note: None,
        series: false,
        default_source: "fleet-ops",
        extra_columns: &["serial", "firmware_id", "valid_from", "valid_to"],
    },
    Table {
        name: "sensors",
        key: "sensor_id",
        node: Some(NodeMap {
            node: SENSOR,
            valid: None,
        }),
        edges: &[],
        note: None,
        series: false,
        default_source: "telemetry",
        extra_columns: &[],
    },
    Table {
        name: "sensor_mounts",
        key: "mount_id",
        node: None,
        edges: &[interval_edge(
            "hw:measured_by",
            "measured_by",
            UNIT_COL,
            End::Column("sensor_id", SENSOR),
        )],
        note: None,
        series: false,
        default_source: "telemetry",
        extra_columns: &["sensor_id", "serial", "valid_from", "valid_to"],
    },
    Table {
        name: "series",
        key: "series_ref_id",
        node: None,
        edges: &[],
        note: None,
        series: true,
        default_source: "telemetry",
        extra_columns: &[
            "sensor_id",
            "store_uri",
            "series_id",
            "start",
            "end",
            "digest",
        ],
    },
    Table {
        name: "maintenance",
        key: "work_order",
        node: Some(NodeMap {
            node: MAINTENANCE,
            valid: Some(("started_at", "ended_at")),
        }),
        edges: &[
            edge(
                "hw:performed_on",
                "performed_on",
                End::Row,
                UNIT_COL,
                Some("started_at"),
                None,
            ),
            EdgeMap {
                when: Some("performed_by"),
                ..edge(
                    "hw:performed_by",
                    "performed_by",
                    End::Row,
                    End::Column("performed_by", USER),
                    Some("started_at"),
                    None,
                )
            },
        ],
        note: Some("note"),
        series: false,
        default_source: "field-service",
        extra_columns: &["serial", "performed_by", "started_at", "ended_at", "note"],
    },
    Table {
        name: "incidents",
        key: "incident_id",
        node: Some(NodeMap {
            node: INCIDENT,
            valid: Some(("opened_at", "closed_at")),
        }),
        edges: &[edge(
            "hw:affected",
            "affected",
            End::Row,
            UNIT_COL,
            Some("opened_at"),
            None,
        )],
        note: Some("description"),
        series: false,
        default_source: "field-service",
        extra_columns: &["serial", "opened_at", "closed_at", "description"],
    },
];

/// Columns every table accepts.
pub const COMMON_COLUMNS: &[&str] = &["readers", "source", "evidence_uri", "evidence_digest"];

/// Look up a table by name.
pub fn table(name: &str) -> Option<&'static Table> {
    TABLES.iter().find(|t| t.name == name)
}

impl Table {
    /// Every column the table accepts.
    pub fn columns(&self) -> Vec<&'static str> {
        let mut columns = vec![self.key];
        if let Some(node) = &self.node {
            columns.extend(node.node.props.iter().map(|p| p.column));
        }
        columns.extend(self.extra_columns.iter().copied());
        columns.extend(COMMON_COLUMNS.iter().copied());
        let mut seen = std::collections::BTreeSet::new();
        columns.retain(|c| seen.insert(*c));
        columns
    }
}

fn props(props: &[Prop]) -> BTreeMap<String, PropertySchema> {
    props
        .iter()
        .map(|p| {
            (
                p.column.to_string(),
                PropertySchema {
                    value_type: p.ty.scalar(),
                    required: p.required,
                    nullable: false,
                },
            )
        })
        .collect()
}

/// The engine graph schema of the reference model. Every node and edge the
/// importer writes is typed against it, so the engine rejects a record that
/// does not fit the model.
pub fn graph_schema() -> GraphSchema {
    GraphSchema {
        id: SCHEMA_ID.into(),
        revision: SCHEMA_REVISION.into(),
        nodes: NODE_TYPES
            .iter()
            .map(|n| {
                (
                    n.type_id.to_string(),
                    NodeSchema {
                        properties: props(n.props),
                        space_id: Some(SPACE.into()),
                        allow_extra_properties: false,
                    },
                )
            })
            .collect(),
        edges: RELATIONS
            .iter()
            .map(|r| {
                (
                    r.predicate.to_string(),
                    EdgeSchema {
                        from_type: r.from.into(),
                        to_type: r.to.into(),
                        properties: props(r.props),
                        allow_cross_space: false,
                        allow_extra_properties: false,
                    },
                )
            })
            .collect(),
    }
}
