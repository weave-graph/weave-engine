//! Tabular input: RFC 4180 CSV, JSON batches and time values.
use crate::schema::{self, Table};
use crate::{error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One input row: column name to text. An empty value is absent.
pub type Row = BTreeMap<String, String>;

/// Most rows one table of one batch may hold.
pub const MAX_ROWS: usize = 1_000_000;
/// Most bytes of one field.
pub const MAX_FIELD_BYTES: usize = 64 * 1024;

/// Rows for one or more tables, imported together.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    /// Claimed provenance source for rows without a `source` column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Rows by table name.
    pub tables: BTreeMap<String, Vec<Row>>,
}

/// One line of an updates file: a batch that is applied as one fact write.
/// On the wire it is a JSON batch with three more fields: `seq`, `kind` and
/// `reported_at`.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub seq: u64,
    /// What happened (`firmware_rollout`, `correction`, ...). Descriptive.
    pub kind: String,
    /// When the source system reported the change. It is not recorded time:
    /// the engine assigns recorded time when the update is written. The
    /// scenario harness uses it to drive a test clock.
    pub reported_at: String,
    pub batch: Batch,
}

impl Update {
    /// Parse one line of an updates file.
    pub fn parse(line: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| error("E_HW_INPUT", &format!("malformed update: {e}")))?;
        let serde_json::Value::Object(mut object) = value else {
            return Err(error("E_HW_INPUT", "an update is an object"));
        };
        let seq = object
            .remove("seq")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| error("E_HW_INPUT", "an update needs an integer seq"))?;
        let mut text = |name: &str| {
            object
                .remove(name)
                .and_then(|v| v.as_str().map(str::to_string))
                .ok_or_else(|| error("E_HW_INPUT", &format!("an update needs a string {name}")))
        };
        let kind = text("kind")?;
        let reported_at = text("reported_at")?;
        parse_time(&reported_at)?;
        Ok(Update {
            seq,
            kind,
            reported_at,
            batch: batch_from_value(serde_json::Value::Object(object))?,
        })
    }

    /// One line of an updates file, without the trailing newline.
    pub fn to_line(&self) -> String {
        let mut value = serde_json::json!({
            "seq": self.seq,
            "kind": self.kind,
            "reported_at": self.reported_at,
            "tables": self.batch.tables,
        });
        if let Some(source) = &self.batch.source {
            value["source"] = source.clone().into();
        }
        value.to_string()
    }
}

/// Parse RFC 4180 CSV for `table`: a header row, then one row per record.
/// Quoted fields may contain commas, quotes (`""`) and line breaks. Every
/// header must be a column of the table, and every record must have exactly
/// as many fields as the header.
pub fn parse_csv(table: &Table, text: &str) -> Result<Vec<Row>> {
    let records = records(text)?;
    let mut records = records.into_iter();
    let Some(header) = records.next() else {
        return Ok(vec![]);
    };
    let allowed = table.columns();
    let mut seen = std::collections::BTreeSet::new();
    for column in &header {
        if !allowed.contains(&column.as_str()) {
            return Err(error(
                "E_HW_INPUT",
                &format!("table {} has no column {column:?}", table.name),
            ));
        }
        if !seen.insert(column.clone()) {
            return Err(error(
                "E_HW_INPUT",
                &format!("table {} repeats column {column:?}", table.name),
            ));
        }
    }
    let mut rows = Vec::new();
    for (line, record) in records.enumerate() {
        if record.len() == 1 && record[0].is_empty() {
            continue;
        }
        if record.len() != header.len() {
            return Err(error(
                "E_HW_INPUT",
                &format!(
                    "table {} record {} has {} fields; the header has {}",
                    table.name,
                    line + 1,
                    record.len(),
                    header.len()
                ),
            ));
        }
        if rows.len() == MAX_ROWS {
            return Err(error("E_BUDGET", "a table holds at most 1000000 rows"));
        }
        rows.push(
            header
                .iter()
                .cloned()
                .zip(record)
                .filter(|(_, v)| !v.is_empty())
                .collect(),
        );
    }
    Ok(rows)
}

fn records(text: &str) -> Result<Vec<Vec<String>>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    let mut at_start = true;
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => {
                    quoted = false;
                    match chars.peek() {
                        None | Some(',') | Some('\n') | Some('\r') => {}
                        Some(_) => {
                            return Err(error("E_HW_INPUT", "text after a closing CSV quote"))
                        }
                    }
                }
                c => field.push(c),
            }
        } else {
            match c {
                '"' if at_start => quoted = true,
                '"' => return Err(error("E_HW_INPUT", "a quote inside an unquoted CSV field")),
                ',' => {
                    record.push(std::mem::take(&mut field));
                    at_start = true;
                    continue;
                }
                '\r' if chars.peek() == Some(&'\n') => continue,
                '\n' | '\r' => {
                    record.push(std::mem::take(&mut field));
                    out.push(std::mem::take(&mut record));
                    at_start = true;
                    continue;
                }
                c => field.push(c),
            }
        }
        if field.len() > MAX_FIELD_BYTES {
            return Err(error("E_BUDGET", "a CSV field holds at most 65536 bytes"));
        }
        at_start = false;
    }
    if quoted {
        return Err(error("E_HW_INPUT", "unterminated CSV quote"));
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        out.push(record);
    }
    Ok(out)
}

/// Write rows as CSV with the given columns. Fields that need it are quoted.
pub fn write_csv(columns: &[&str], rows: &[Row]) -> String {
    fn escape(value: &str) -> String {
        if value.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", value.replace('"', "\"\""))
        } else {
            value.to_string()
        }
    }
    let mut out = columns
        .iter()
        .map(|c| escape(c))
        .collect::<Vec<_>>()
        .join(",");
    out.push('\n');
    for row in rows {
        let line = columns
            .iter()
            .map(|c| escape(row.get(*c).map(String::as_str).unwrap_or("")))
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Parse a JSON batch: `{"source"?: "...", "tables": {"units": [{...}]}}`.
/// Values may be strings, numbers, booleans or null (absent).
pub fn parse_json_batch(text: &str) -> Result<Batch> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| error("E_HW_INPUT", &format!("malformed JSON batch: {e}")))?;
    batch_from_value(value)
}

/// Normalize a JSON value into a batch, checking table and column names.
pub fn batch_from_value(value: serde_json::Value) -> Result<Batch> {
    let mut object = match value {
        serde_json::Value::Object(o) => o,
        _ => return Err(error("E_HW_INPUT", "a JSON batch is an object")),
    };
    let source = match object.remove("source") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s),
        Some(_) => return Err(error("E_HW_INPUT", "batch source must be a string")),
    };
    let tables = match object.remove("tables") {
        Some(serde_json::Value::Object(t)) => t,
        _ => return Err(error("E_HW_INPUT", "a JSON batch needs a tables object")),
    };
    if let Some(extra) = object.keys().next() {
        return Err(error(
            "E_HW_INPUT",
            &format!("unknown JSON batch field {extra:?}"),
        ));
    }
    let mut out = Batch {
        source,
        tables: BTreeMap::new(),
    };
    for (name, rows) in tables {
        let table = schema::table(&name)
            .ok_or_else(|| error("E_HW_INPUT", &format!("unknown table {name:?}")))?;
        let serde_json::Value::Array(rows) = rows else {
            return Err(error("E_HW_INPUT", "table rows must be an array"));
        };
        if rows.len() > MAX_ROWS {
            return Err(error("E_BUDGET", "a table holds at most 1000000 rows"));
        }
        let allowed = table.columns();
        let mut parsed = Vec::with_capacity(rows.len());
        for row in rows {
            let serde_json::Value::Object(row) = row else {
                return Err(error("E_HW_INPUT", "a row is an object"));
            };
            let mut out_row = Row::new();
            for (column, value) in row {
                if !allowed.contains(&column.as_str()) {
                    return Err(error(
                        "E_HW_INPUT",
                        &format!("table {name} has no column {column:?}"),
                    ));
                }
                let text = match value {
                    serde_json::Value::Null => continue,
                    serde_json::Value::String(s) => s,
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    serde_json::Value::Array(items) if column == "readers" => items
                        .iter()
                        .map(|i| {
                            i.as_str().map(str::to_string).ok_or_else(|| {
                                error("E_HW_INPUT", "readers must be a list of strings")
                            })
                        })
                        .collect::<Result<Vec<_>>>()?
                        .join(";"),
                    _ => {
                        return Err(error(
                            "E_HW_INPUT",
                            &format!("column {column:?} needs a scalar value"),
                        ))
                    }
                };
                if text.len() > MAX_FIELD_BYTES {
                    return Err(error("E_BUDGET", "a field holds at most 65536 bytes"));
                }
                if !text.is_empty() {
                    out_row.insert(column, text);
                }
            }
            parsed.push(out_row);
        }
        out.tables.insert(name, parsed);
    }
    Ok(out)
}

/// Parse a time: integer Unix-epoch milliseconds, `YYYY-MM-DD`, or
/// `YYYY-MM-DDTHH:MM:SS[.mmm]Z` (UTC only).
pub fn parse_time(text: &str) -> Result<i64> {
    if let Ok(ms) = text.parse::<i64>() {
        return Ok(ms);
    }
    let bad = || {
        error(
            "E_HW_TIME",
            &format!("{text:?} is not epoch milliseconds or a UTC ISO 8601 date-time"),
        )
    };
    let bytes = text.as_bytes();
    let num = |range: std::ops::Range<usize>| -> Result<i64> {
        let part = text.get(range).ok_or_else(bad)?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        part.parse::<i64>().map_err(|_| bad())
    };
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return Err(bad());
    }
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return Err(bad());
    }
    let mut ms = days_from_civil(year, month, day) * 86_400_000;
    if bytes.len() == 10 {
        return Ok(ms);
    }
    if bytes.len() < 20 || bytes[10] != b'T' || bytes[13] != b':' || bytes[16] != b':' {
        return Err(bad());
    }
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if hour > 23 || minute > 59 || second > 59 {
        return Err(bad());
    }
    ms += ((hour * 60 + minute) * 60 + second) * 1000;
    let rest = &text[19..];
    let rest = if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.trim_end_matches('Z');
        if digits.len() != 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        ms += digits.parse::<i64>().map_err(|_| bad())?;
        &fraction[3..]
    } else {
        rest
    };
    if rest != "Z" {
        return Err(bad());
    }
    Ok(ms)
}

/// Format Unix-epoch milliseconds as `YYYY-MM-DDTHH:MM:SSZ` (dropping
/// milliseconds) or `YYYY-MM-DD` at midnight.
pub fn format_time(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rest = ms.rem_euclid(86_400_000) / 1000;
    let (y, m, d) = civil_from_days(days);
    if rest == 0 {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            rest / 3600,
            rest / 60 % 60,
            rest % 60
        )
    }
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Howard Hinnant's civil-date algorithms.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_round_trip() {
        assert_eq!(parse_time("1970-01-01").unwrap(), 0);
        assert_eq!(parse_time("2024-02-29").unwrap(), 1_709_164_800_000);
        assert_eq!(
            parse_time("2024-02-29T12:30:05Z").unwrap(),
            1_709_209_805_000
        );
        assert_eq!(
            parse_time("2024-02-29T12:30:05.250Z").unwrap(),
            1_709_209_805_250
        );
        assert_eq!(parse_time("-5").unwrap(), -5);
        for bad in [
            "2023-02-29",
            "2024-13-01",
            "2024-01-01T25:00:00Z",
            "2024-01-01T00:00:00+01:00",
            "yesterday",
        ] {
            assert_eq!(parse_time(bad).unwrap_err().code, "E_HW_TIME", "{bad}");
        }
        for ms in [0, 1_709_164_800_000, 1_709_209_805_000, -86_400_000] {
            assert_eq!(parse_time(&format_time(ms)).unwrap(), ms);
        }
    }

    #[test]
    fn csv_quotes_commas_and_line_breaks() {
        let table = schema::table("maintenance").unwrap();
        let text = "work_order,kind,serial,started_at,note\r\nWO-1,repair,SN-1,2024-01-01,\"Bearing noise, \"\"grinding\"\"\nreplaced\"\nWO-2,inspection,SN-1,2024-02-01,\n";
        let rows = parse_csv(table, text).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["note"], "Bearing noise, \"grinding\"\nreplaced");
        assert!(!rows[1].contains_key("note"));
        let columns = ["work_order", "kind", "serial", "started_at", "note"];
        assert_eq!(parse_csv(table, &write_csv(&columns, &rows)).unwrap(), rows);
        assert_eq!(
            parse_csv(table, "work_order,colour\nWO-1,red\n")
                .unwrap_err()
                .code,
            "E_HW_INPUT"
        );
        assert_eq!(
            parse_csv(table, "work_order,kind\nWO-1\n")
                .unwrap_err()
                .code,
            "E_HW_INPUT"
        );
        assert_eq!(
            parse_csv(table, "work_order,kind\n\"WO-1,repair\n")
                .unwrap_err()
                .code,
            "E_HW_INPUT"
        );
    }
}
