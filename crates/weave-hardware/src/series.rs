//! `SeriesRef`: a reference to a time range of an external series.
//!
//! Raw samples never enter the graph (roadmap C1 leaning). A sensor carries
//! one `hw:series` metadata attachment per referenced range. Its value is a
//! typed literal (`type: "weave.hw.series_ref"`, `version: 1`), and its valid
//! time equals the referenced range, so the engine's valid-time selection
//! and the overlap helpers in [`crate::query`] apply to it directly.
use crate::schema::{NOTE_KEY, SERIES_KEY};
use crate::{error, Result};
use serde::{Deserialize, Serialize};
use weave_contract::provenance::ClaimedProvenance;
use weave_contract::{Interval, MetadataAttachment, MetadataHost, MetadataValue};

/// Literal type tag of a serialized `SeriesRef`.
pub const SERIES_REF_TYPE: &str = "weave.hw.series_ref";
/// Wire version of a serialized `SeriesRef`.
pub const SERIES_REF_VERSION: u32 = 1;
/// Maximum UTF-8 bytes of a series URI, series ID or digest.
pub const MAX_SERIES_FIELD: usize = 2048;

/// A half-open time range `[start, end)` of one series in an external store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesRef {
    /// Location of the series store (for example a Parquet dataset or a TSDB).
    pub store_uri: String,
    /// Series identifier inside the store.
    pub series_id: String,
    /// Referenced range, Unix-epoch milliseconds, `end` exclusive.
    pub start: i64,
    pub end: i64,
    /// Digest of the referenced samples (for example `sha256:…`), so a
    /// reader can detect that the store no longer holds the same data.
    pub digest: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    #[serde(rename = "type")]
    kind: String,
    version: u32,
    store_uri: String,
    series_id: String,
    range: Interval,
    digest: String,
}

fn field(name: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_SERIES_FIELD {
        return Err(error(
            "E_HW_SERIES",
            &format!("series {name} needs 1 to {MAX_SERIES_FIELD} bytes"),
        ));
    }
    Ok(())
}

impl SeriesRef {
    pub fn validate(&self) -> Result<()> {
        field("store_uri", &self.store_uri)?;
        field("series_id", &self.series_id)?;
        field("digest", &self.digest)?;
        if self.start >= self.end {
            return Err(error("E_HW_SERIES", "series range end must exceed start"));
        }
        Ok(())
    }

    /// The referenced range as an interval.
    pub fn range(&self) -> Interval {
        Interval {
            start: self.start,
            end: Some(self.end),
        }
    }

    /// True when the referenced range and `[start, end)` share an instant.
    /// `end: None` is unbounded.
    pub fn overlaps(&self, start: i64, end: Option<i64>) -> bool {
        self.start < end.unwrap_or(i64::MAX) && start < self.end
    }

    /// The typed metadata value.
    pub fn to_value(&self) -> Result<MetadataValue> {
        self.validate()?;
        Ok(MetadataValue::Literal {
            value: serde_json::to_value(Wire {
                kind: SERIES_REF_TYPE.into(),
                version: SERIES_REF_VERSION,
                store_uri: self.store_uri.clone(),
                series_id: self.series_id.clone(),
                range: self.range(),
                digest: self.digest.clone(),
            })
            .map_err(|e| error("E_HW_SERIES", &e.to_string()))?,
        })
    }

    /// Decode a typed metadata value.
    pub fn from_value(value: &MetadataValue) -> Result<Self> {
        let MetadataValue::Literal { value } = value else {
            return Err(error(
                "E_HW_SERIES",
                "a series reference is a literal value",
            ));
        };
        let wire: Wire = serde_json::from_value(value.clone())
            .map_err(|e| error("E_HW_SERIES", &format!("malformed series reference: {e}")))?;
        if wire.kind != SERIES_REF_TYPE || wire.version != SERIES_REF_VERSION {
            return Err(error(
                "E_HW_SERIES",
                "unsupported series reference type or version",
            ));
        }
        let end = wire
            .range
            .end
            .ok_or_else(|| error("E_HW_SERIES", "a series range must be bounded"))?;
        let series = SeriesRef {
            store_uri: wire.store_uri,
            series_id: wire.series_id,
            start: wire.range.start,
            end,
            digest: wire.digest,
        };
        series.validate()?;
        Ok(series)
    }

    /// The `hw:series` attachment that places this reference on a sensor.
    pub fn to_attachment(
        &self,
        id: &str,
        sensor_node: &str,
        readers: Vec<String>,
        provenance: Option<ClaimedProvenance>,
    ) -> Result<MetadataAttachment> {
        let attachment = serde_json::json!({
            "id": id,
            "host": {"kind": "node", "id": sensor_node},
            "key": SERIES_KEY,
            "value": self.to_value()?,
            "valid_time": self.range(),
            "provenance": provenance,
            "readers": readers,
        });
        serde_json::from_value(attachment).map_err(|e| error("E_HW_SERIES", &e.to_string()))
    }

    /// Decode an `hw:series` attachment. Its valid time must equal the range.
    pub fn from_attachment(attachment: &MetadataAttachment) -> Result<Self> {
        if attachment.key != SERIES_KEY {
            return Err(error(
                "E_HW_SERIES",
                "attachment is not an hw:series reference",
            ));
        }
        let series = Self::from_value(&attachment.value)?;
        if attachment.valid_time != series.range() {
            return Err(error(
                "E_HW_SERIES",
                "series attachment valid time differs from its range",
            ));
        }
        Ok(series)
    }
}

/// The text of an `hw:note` attachment (maintenance notes, incident
/// descriptions): the stable hook A4 embeddings attach to.
pub fn note_text(attachment: &MetadataAttachment) -> Option<&str> {
    match (&attachment.key, &attachment.value, &attachment.host) {
        (key, MetadataValue::Literal { value }, MetadataHost::Node { .. }) if key == NOTE_KEY => {
            value.as_str()
        }
        _ => None,
    }
}
