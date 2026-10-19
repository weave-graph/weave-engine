//! `SeriesRef` round-trips through the store and is queryable by half-open
//! time-range overlap, with reader restrictions and recorded-state pins.
use serde_json::json;
use weave_contract::{MetadataAttachment, MetadataValue};
use weave_engine::{Engine, HostContext};
use weave_hardware::query::{series_overlapping, At};
use weave_hardware::table::batch_from_value;
use weave_hardware::{import_batch, SeriesRef, Target};

fn writer() -> HostContext {
    HostContext::new("etl", ["fleet".to_string()])
}
fn reader(who: &str) -> HostContext {
    HostContext::new(who, Vec::<String>::new())
}

fn series_row(id: &str, sensor: &str, start: i64, end: i64) -> serde_json::Value {
    json!({"series_ref_id": id, "sensor_id": sensor, "store_uri": "parquet://telemetry/vibration",
           "series_id": sensor, "start": start, "end": end, "digest": format!("sha256:{id}")})
}

fn fleet() -> (Engine, String) {
    let mut e = Engine::memory().unwrap();
    let batch = batch_from_value(json!({"tables": {
        "models": [{"model_id": "M", "name": "M"}],
        "units": [
            {"serial": "A", "model_id": "M", "commissioned_at": 0},
            {"serial": "B", "model_id": "M", "commissioned_at": 0},
            {"serial": "C", "model_id": "M", "commissioned_at": 0, "readers": ["owner"]}
        ],
        "sensors": [
            {"sensor_id": "X", "kind": "vibration"},
            {"sensor_id": "Y", "kind": "vibration", "readers": ["owner"]}
        ],
        "sensor_mounts": [
            {"mount_id": "X1", "sensor_id": "X", "serial": "A", "valid_from": 0, "valid_to": 15},
            {"mount_id": "X2", "sensor_id": "X", "serial": "B", "valid_from": 15},
            {"mount_id": "Y1", "sensor_id": "Y", "serial": "C", "valid_from": 0, "readers": ["owner"]}
        ],
        "series": [
            series_row("R1", "X", 0, 10),
            series_row("R2", "X", 10, 20),
            series_row("R3", "X", 30, 40),
            {"series_ref_id": "R4", "sensor_id": "Y", "store_uri": "tsdb://plant/vib", "series_id": "Y",
             "start": 0, "end": 50, "digest": "sha256:r4", "readers": ["owner"]}
        ]
    }}))
    .unwrap();
    let report = import_batch(&mut e, &writer(), &Target::new("fleet"), &batch).unwrap();
    (e, report.head.unwrap())
}

fn ids(
    e: &mut Engine,
    who: &str,
    unit: Option<&str>,
    start: i64,
    end: Option<i64>,
    at: &At,
) -> Vec<(String, Vec<String>)> {
    let (hits, _) = series_overlapping(e, &reader(who), "fleet", unit, start, end, at).unwrap();
    hits.into_iter()
        .map(|h| (h.id.trim_start_matches("series:").to_string(), h.units))
        .collect()
}
fn s(items: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    items
        .iter()
        .map(|(id, units)| {
            (
                id.to_string(),
                units.iter().map(|u| u.to_string()).collect(),
            )
        })
        .collect()
}

#[test]
fn series_ref_round_trips() {
    let series = SeriesRef {
        store_uri: "parquet://fleet-telemetry/vibration/year=2024".into(),
        series_id: "SN-000001-V".into(),
        start: 1_704_067_200_000,
        end: 1_719_792_000_000,
        digest: "sha256:abc".into(),
    };
    // The wire form is pinned.
    let MetadataValue::Literal { value } = series.to_value().unwrap() else {
        panic!("literal expected")
    };
    assert_eq!(
        value,
        json!({"type": "weave.hw.series_ref", "version": 1,
               "store_uri": "parquet://fleet-telemetry/vibration/year=2024",
               "series_id": "SN-000001-V",
               "range": {"start": 1_704_067_200_000i64, "end": 1_719_792_000_000i64},
               "digest": "sha256:abc"})
    );
    let attachment = series
        .to_attachment("series:1", "sensor:SN-000001-V", vec![], None)
        .unwrap();
    assert_eq!(attachment.valid_time, series.range());
    assert_eq!(SeriesRef::from_attachment(&attachment).unwrap(), series);
    let text = serde_json::to_string(&attachment).unwrap();
    let back: MetadataAttachment = serde_json::from_str(&text).unwrap();
    assert_eq!(SeriesRef::from_attachment(&back).unwrap(), series);

    // Through the store: written by the importer, read back unchanged.
    let (mut e, _) = fleet();
    let (hits, coverage) = series_overlapping(
        &mut e,
        &reader("owner"),
        "fleet",
        None,
        i64::MIN / 2,
        None,
        &At::Head,
    )
    .unwrap();
    assert_eq!(coverage, weave_contract::Coverage::Complete);
    let r4 = hits.iter().find(|h| h.id == "series:R4").unwrap();
    assert_eq!(
        r4.series,
        SeriesRef {
            store_uri: "tsdb://plant/vib".into(),
            series_id: "Y".into(),
            start: 0,
            end: 50,
            digest: "sha256:r4".into(),
        }
    );
    assert_eq!(r4.sensor, "sensor:Y");
}

#[test]
fn malformed_series_refs_are_rejected() {
    let good = SeriesRef {
        store_uri: "s".into(),
        series_id: "i".into(),
        start: 0,
        end: 1,
        digest: "d".into(),
    };
    for bad in [
        SeriesRef {
            store_uri: String::new(),
            ..good.clone()
        },
        SeriesRef {
            series_id: String::new(),
            ..good.clone()
        },
        SeriesRef {
            digest: String::new(),
            ..good.clone()
        },
        SeriesRef {
            start: 1,
            ..good.clone()
        },
    ] {
        assert_eq!(bad.to_value().unwrap_err().code, "E_HW_SERIES");
    }
    let literal = |v: serde_json::Value| MetadataValue::Literal { value: v };
    let wire = json!({"type": "weave.hw.series_ref", "version": 1, "store_uri": "s",
                      "series_id": "i", "range": {"start": 0, "end": 1}, "digest": "d"});
    assert_eq!(SeriesRef::from_value(&literal(wire.clone())).unwrap(), good);
    for (field, value) in [
        ("type", json!("other")),
        ("version", json!(2)),
        ("range", json!({"start": 0, "end": null})),
        ("range", json!({"start": 2, "end": 1})),
        ("extra", json!(true)),
    ] {
        let mut v = wire.clone();
        v[field] = value;
        assert_eq!(
            SeriesRef::from_value(&literal(v)).unwrap_err().code,
            "E_HW_SERIES",
            "{field}"
        );
    }
    let mut attachment = good.to_attachment("a", "sensor:s", vec![], None).unwrap();
    attachment.valid_time.end = Some(2);
    assert_eq!(
        SeriesRef::from_attachment(&attachment).unwrap_err().code,
        "E_HW_SERIES"
    );
    attachment.valid_time.end = Some(1);
    attachment.key = "hw:note".into();
    assert_eq!(
        SeriesRef::from_attachment(&attachment).unwrap_err().code,
        "E_HW_SERIES"
    );
}

#[test]
fn series_are_queryable_by_half_open_overlap() {
    let (mut e, _) = fleet();
    let head = At::Head;
    // [10, 30): R1 ends at 10 and R3 starts at 30, so neither overlaps.
    assert_eq!(
        ids(&mut e, "analyst", None, 10, Some(30), &head),
        s(&[("R2", &["A", "B"])])
    );
    assert_eq!(
        ids(&mut e, "analyst", None, 9, Some(11), &head),
        s(&[("R1", &["A"]), ("R2", &["A"])])
    );
    assert_eq!(
        ids(&mut e, "analyst", None, 35, None, &head),
        s(&[("R3", &["B"])])
    );
    assert_eq!(ids(&mut e, "analyst", None, 20, Some(30), &head), s(&[]));
    // Per unit: sensor X left unit A at 15, so A has R2 only before then.
    assert_eq!(
        ids(&mut e, "analyst", Some("A"), 16, Some(30), &head),
        s(&[])
    );
    assert_eq!(
        ids(&mut e, "analyst", Some("A"), 12, Some(30), &head),
        s(&[("R2", &["A"])])
    );
    assert_eq!(
        ids(&mut e, "analyst", Some("B"), 0, None, &head),
        s(&[("R2", &["B"]), ("R3", &["B"])])
    );
    // The restricted series exists only for its reader.
    assert_eq!(ids(&mut e, "analyst", Some("C"), 0, None, &head), s(&[]));
    assert_eq!(
        ids(&mut e, "owner", Some("C"), 0, None, &head),
        s(&[("R4", &["C"])])
    );
    assert_eq!(
        series_overlapping(&mut e, &reader("a"), "fleet", None, 5, Some(5), &head)
            .unwrap_err()
            .code,
        "E_INTERVAL"
    );
}

#[test]
fn series_queries_respect_the_selected_revision() {
    let (mut e, r1) = fleet();
    let batch =
        batch_from_value(json!({"tables": {"series": [series_row("R5", "X", 40, 60)]}})).unwrap();
    import_batch(&mut e, &writer(), &Target::new("fleet"), &batch).unwrap();
    assert_eq!(
        ids(&mut e, "analyst", None, 45, None, &At::Head),
        s(&[("R5", &["B"])])
    );
    assert_eq!(
        ids(&mut e, "analyst", None, 45, None, &At::Revision(r1)),
        s(&[])
    );
}
