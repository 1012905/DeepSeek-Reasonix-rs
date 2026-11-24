//! The chart spec contract (Go `internal/contract/chartspec/`): decode,
//! validate, summarize and canonicalize a `render_chart` argument. Pure
//! functions of the spec bytes; nothing draws, fetches or writes.
//!
//! Parity notes: date-layout parsing covers RFC3339, date-time, date, month
//! and year shapes; label control/invisible-category checks keep the control
//! char fence. Full grapheme/bidi label fidelity stays with the host renderer.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// `chartspec.ToolName`.
pub const TOOL_NAME: &str = "render_chart";

pub const SPEC_VERSION: i64 = 1;
pub const MAX_SPEC_BYTES: usize = 256 << 10;
pub const MAX_ROWS: usize = 5000;
pub const MAX_COLUMNS: usize = 32;
pub const MAX_SERIES: usize = 12;
pub const MAX_POINTS: usize = 2000;
pub const MAX_MARKS: usize = 8;
pub const MAX_LABEL_RUNES: usize = 64;
const MAX_SAFE_INT: i64 = 1 << 53;

// --- refusal identities (Go `chartspec/errors.go`) ---------------------------

pub const CODE_SPEC_VERSION_UNSUPPORTED: &str = "chart.spec_version_unsupported";
pub const CODE_SCHEMA_INVALID: &str = "chart.schema_invalid";
pub const CODE_LIMIT_EXCEEDED: &str = "chart.limit_exceeded";
pub const CODE_COLUMN_UNKNOWN: &str = "chart.column_unknown";
pub const CODE_TYPE_MISMATCH: &str = "chart.type_mismatch";

pub const CAP_BYTES: &str = "bytes";
pub const CAP_ROWS: &str = "rows";
pub const CAP_COLUMNS: &str = "columns";
pub const CAP_SERIES: &str = "series";
pub const CAP_POINTS: &str = "points";
pub const CAP_MARKS: &str = "marks";
pub const CAP_LABEL: &str = "label";

/// A chartspec refusal with an identity. `path` is a JSON pointer; `cap` is
/// set only for limit refusals (Go `chartspec.Error`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartError {
    pub code: String,
    pub path: String,
    pub cap: String,
}

impl ChartError {
    fn new(code: &str, path: &str) -> Self {
        Self {
            code: code.to_owned(),
            path: path.to_owned(),
            cap: String::new(),
        }
    }

    fn exceeded(cap: &str, path: &str) -> Self {
        Self {
            code: CODE_LIMIT_EXCEEDED.to_owned(),
            path: path.to_owned(),
            cap: cap.to_owned(),
        }
    }
}

impl std::fmt::Display for ChartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.code)?;
        if !self.cap.is_empty() {
            write!(f, " cap={}", self.cap)?;
        }
        if !self.path.is_empty() {
            write!(f, " at {}", self.path)?;
        }
        Ok(())
    }
}

impl std::error::Error for ChartError {}

// --- model -------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CellKind {
    #[default]
    Null,
    Number,
    String,
}

/// One scalar of a row. `lossy` marks a number float64 cannot carry exactly;
/// validation refuses it (Go `chartspec.Cell`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cell {
    pub kind: CellKind,
    pub num: f64,
    pub str: String,
    pub lossy: bool,
}

impl<'de> Deserialize<'de> for Cell {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(number_cell(&value))
    }
}

impl Serialize for Cell {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.kind {
            CellKind::Null => s.serialize_none(),
            CellKind::Number => s.serialize_f64(self.num),
            CellKind::String => s.serialize_str(&self.str),
        }
    }
}

fn number_cell(value: &Value) -> Cell {
    match value {
        Value::Null => Cell::default(),
        Value::String(s) => Cell {
            kind: CellKind::String,
            str: s.clone(),
            ..Default::default()
        },
        Value::Number(n) => {
            let lit = n.to_string();
            let f = n.as_f64().unwrap_or(f64::NAN);
            let mut lossy = f.is_nan() || f.is_infinite();
            if !lossy && is_integer_literal(&lit) {
                if let Ok(parsed) = lit.parse::<i64>() {
                    lossy = !(-MAX_SAFE_INT..=MAX_SAFE_INT).contains(&parsed);
                } else {
                    lossy = true;
                }
            }
            let num = if lossy { 0.0 } else { f };
            Cell {
                kind: CellKind::Number,
                num,
                lossy,
                ..Default::default()
            }
        }
        _ => Cell {
            kind: CellKind::String,
            str: value.to_string(),
            ..Default::default()
        },
    }
}

fn is_integer_literal(lit: &str) -> bool {
    !lit.chars().any(|c| c == '.' || c == 'e' || c == 'E')
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub column_type: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Cell>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Mark {
    #[serde(rename = "type")]
    pub kind: String,
    pub x: String,
    pub y: Vec<String>,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub stacked: bool,
    #[serde(default)]
    pub donut: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Axis {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub scale: String,
    #[serde(default)]
    pub format: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    #[serde(rename = "spec_version")]
    pub spec_version: i64,
    #[serde(default)]
    pub title: String,
    pub data: Data,
    pub marks: Vec<Mark>,
    #[serde(rename = "x_axis")]
    pub x_axis: Option<Axis>,
    #[serde(rename = "y_axis")]
    pub y_axis: Option<Axis>,
}

// --- parse -------------------------------------------------------------------

/// Decodes and validates a spec. The size cap is checked before decoding,
/// duplicate keys are refused, and nothing is returned on error (Go
/// `chartspec.Parse`).
pub fn parse(raw: &[u8]) -> Result<Spec, ChartError> {
    if raw.len() > MAX_SPEC_BYTES {
        return Err(ChartError::exceeded(CAP_BYTES, ""));
    }
    reject_duplicate_keys(raw)?;
    let spec: Spec =
        Spec::deserialize(&mut serde_json::Deserializer::from_slice(raw)).map_err(decode_error)?;
    spec.validate()?;
    Ok(spec)
}

fn decode_error(_err: serde_json::Error) -> ChartError {
    ChartError::new(CODE_SCHEMA_INVALID, "")
}

/// Refuses an object that repeats a key, case-folded because the decoder keeps
/// the last one (Go `rejectDuplicateKeys`). Iterative over a raw token walk so
/// nesting depth costs no stack.
fn reject_duplicate_keys(raw: &[u8]) -> Result<(), ChartError> {
    let text = std::str::from_utf8(raw).map_err(|_| ChartError::new(CODE_SCHEMA_INVALID, ""))?;
    let chars: Vec<char> = text.chars().collect();
    let mut stack: Vec<Option<std::collections::HashSet<String>>> = Vec::new();
    let mut expect_key = false;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '{' => {
                stack.push(Some(std::collections::HashSet::new()));
                expect_key = true;
            }
            '[' => {
                stack.push(None);
                expect_key = false;
            }
            '}' | ']' => {
                stack.pop();
                expect_key = stack.last().is_some_and(|o| o.is_some());
            }
            '"' => {
                let (s, next) = read_json_string(&chars, i);
                if let Some(Some(keys)) = stack.last_mut() {
                    if expect_key {
                        let folded = s.to_lowercase();
                        if keys.contains(&folded) {
                            return Err(ChartError::new(CODE_SCHEMA_INVALID, ""));
                        }
                        keys.insert(folded);
                        expect_key = false;
                    } else {
                        expect_key = true;
                    }
                }
                i = next;
                continue;
            }
            ':' => expect_key = false,
            ',' => {
                if let Some(Some(_)) = stack.last() {
                    expect_key = true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    Ok(())
}

fn read_json_string(chars: &[char], start: usize) -> (String, usize) {
    let mut out = String::new();
    let mut i = start + 1;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '"' {
            return (out, i + 1);
        }
        out.push(c);
        i += 1;
    }
    (out, i)
}

// --- validate ----------------------------------------------------------------

impl Spec {
    /// Validates structure, types and caps; first refusal wins (Go
    /// `Spec.Validate`).
    pub fn validate(&self) -> Result<(), ChartError> {
        if self.spec_version == 0 {
            return Err(ChartError::new(CODE_SCHEMA_INVALID, "/spec_version"));
        }
        if self.spec_version != SPEC_VERSION {
            return Err(ChartError::new(
                CODE_SPEC_VERSION_UNSUPPORTED,
                "/spec_version",
            ));
        }
        check_label(&self.title, "/title")?;
        validate_axis(self.x_axis.as_ref(), "/x_axis")?;
        validate_axis(self.y_axis.as_ref(), "/y_axis")?;
        let cols = self.data.validate()?;
        if self.marks.is_empty() {
            return Err(ChartError::new(CODE_SCHEMA_INVALID, "/marks"));
        }
        if self.marks.len() > MAX_MARKS {
            return Err(ChartError::exceeded(CAP_MARKS, "/marks"));
        }
        for (i, mark) in self.marks.iter().enumerate() {
            mark.validate(
                &format!("/marks/{i}"),
                &cols,
                &self.data,
                self.y_axis.as_ref(),
            )?;
        }
        Ok(())
    }

    /// Canonical bytes: struct field order, no whitespace, so equal specs give
    /// equal bytes (Go `Spec.Canonical`).
    pub fn canonical(&self) -> Result<Vec<u8>, ChartError> {
        serde_json::to_vec(self).map_err(|_| ChartError::new(CODE_SCHEMA_INVALID, ""))
    }
}

fn validate_axis(axis: Option<&Axis>, path: &str) -> Result<(), ChartError> {
    let Some(axis) = axis else { return Ok(()) };
    check_label(&axis.title, &format!("{path}/title"))?;
    check_label(&axis.unit, &format!("{path}/unit"))?;
    match axis.scale.as_str() {
        "" | "linear" | "log" => {}
        _ => {
            return Err(ChartError::new(
                CODE_SCHEMA_INVALID,
                &format!("{path}/scale"),
            ));
        }
    }
    match axis.format.as_str() {
        "" | "integer" | "decimal" | "percent" => {}
        _ => {
            return Err(ChartError::new(
                CODE_SCHEMA_INVALID,
                &format!("{path}/format"),
            ));
        }
    }
    Ok(())
}

impl Data {
    fn validate(&self) -> Result<std::collections::HashMap<String, usize>, ChartError> {
        if self.columns.is_empty() {
            return Err(ChartError::new(CODE_SCHEMA_INVALID, "/data/columns"));
        }
        if self.columns.len() > MAX_COLUMNS {
            return Err(ChartError::exceeded(CAP_COLUMNS, "/data/columns"));
        }
        if self.rows.len() > MAX_ROWS {
            return Err(ChartError::exceeded(CAP_ROWS, "/data/rows"));
        }
        let mut index = std::collections::HashMap::new();
        for (i, col) in self.columns.iter().enumerate() {
            let path = format!("/data/columns/{i}");
            if col.name.is_empty() {
                return Err(ChartError::new(
                    CODE_SCHEMA_INVALID,
                    &format!("{path}/name"),
                ));
            }
            check_label(&col.name, &format!("{path}/name"))?;
            if index.insert(col.name.clone(), i).is_some() {
                return Err(ChartError::new(
                    CODE_SCHEMA_INVALID,
                    &format!("{path}/name"),
                ));
            }
            match col.column_type.as_str() {
                "number" | "string" | "date" => {}
                _ => {
                    return Err(ChartError::new(
                        CODE_SCHEMA_INVALID,
                        &format!("{path}/type"),
                    ));
                }
            }
        }
        for (r, row) in self.rows.iter().enumerate() {
            let row_path = format!("/data/rows/{r}");
            if row.len() != self.columns.len() {
                return Err(ChartError::new(CODE_SCHEMA_INVALID, &row_path));
            }
            for (c, cell) in row.iter().enumerate() {
                cell.validate(&self.columns[c].column_type, &format!("{row_path}/{c}"))?;
            }
        }
        Ok(index)
    }
}

impl Cell {
    fn validate(&self, column_type: &str, path: &str) -> Result<(), ChartError> {
        match self.kind {
            CellKind::Null => Ok(()),
            CellKind::Number => {
                if column_type != "number" || self.lossy {
                    return Err(ChartError::new(CODE_TYPE_MISMATCH, path));
                }
                Ok(())
            }
            CellKind::String => {
                if column_type == "number" {
                    return Err(ChartError::new(CODE_TYPE_MISMATCH, path));
                }
                check_label(&self.str, path)?;
                if column_type == "date" && !is_date(&self.str) {
                    return Err(ChartError::new(CODE_TYPE_MISMATCH, path));
                }
                Ok(())
            }
        }
    }
}

fn is_date(s: &str) -> bool {
    if rfc3339_shape(s).is_some() {
        return true;
    }
    // 2006-01-02T15:04:05 / 2006-01-02 / 2006-01 / 2006
    let parts: Vec<&str> = s.split('-').collect();
    match parts.len() {
        1 => s.len() == 4 && s.chars().all(|c| c.is_ascii_digit()),
        2 => s.len() == 7 && parts[0].len() == 4 && parts[1].len() == 2,
        3 => {
            let day: Vec<&str> = parts[2].split('T').collect();
            day.len() == 1 && day[0].len() == 2
        }
        _ => false,
    }
}

/// RFC3339 shape check without pulling a calendar crate into tools: date and
/// time digit shapes plus the `Z`/`±hh:mm` offset. Calendar-validity parsing
/// stays a host-side parity gap.
fn rfc3339_shape(s: &str) -> Option<()> {
    let (date, time) = s.split_once('T')?;
    let d: Vec<&str> = date.split('-').collect();
    if d.len() != 3 || d[0].len() != 4 || d[1].len() != 2 || d[2].len() != 2 {
        return None;
    }
    let (clock, offset) = match time.strip_suffix('Z') {
        Some(clock) => (clock, None),
        None => {
            let idx = time.rfind(['+', '-'])?;
            let (clock, off) = time.split_at(idx);
            let (hh, mm) = off[1..].split_once(':')?;
            if hh.len() != 2 || mm.len() != 2 {
                return None;
            }
            (clock, Some(off))
        }
    };
    let hms: Vec<&str> = clock.split(':').collect();
    if hms.len() != 3 || hms[0].len() != 2 || hms[1].len() != 2 || hms[2].len() != 2 {
        return None;
    }
    let _ = offset;
    Some(())
}

impl Mark {
    fn validate(
        &self,
        path: &str,
        cols: &std::collections::HashMap<String, usize>,
        data: &Data,
        y_axis: Option<&Axis>,
    ) -> Result<(), ChartError> {
        match self.kind.as_str() {
            "bar" | "line" | "pie" => {}
            _ => {
                return Err(ChartError::new(
                    CODE_SCHEMA_INVALID,
                    &format!("{path}/type"),
                ));
            }
        }
        if (self.stacked && self.kind != "bar") || (self.donut && self.kind != "pie") {
            return Err(ChartError::new(CODE_SCHEMA_INVALID, path));
        }
        if !cols.contains_key(&self.x) {
            return Err(ChartError::new(CODE_COLUMN_UNKNOWN, &format!("{path}/x")));
        }
        if self.y.is_empty()
            || (self.kind == "pie" && (self.y.len() != 1 || !self.color.is_empty()))
        {
            return Err(ChartError::new(CODE_SCHEMA_INVALID, &format!("{path}/y")));
        }
        for (i, y) in self.y.iter().enumerate() {
            let Some(&idx) = cols.get(y) else {
                return Err(ChartError::new(
                    CODE_COLUMN_UNKNOWN,
                    &format!("{path}/y/{i}"),
                ));
            };
            if data.columns[idx].column_type != "number" {
                return Err(ChartError::new(
                    CODE_TYPE_MISMATCH,
                    &format!("{path}/y/{i}"),
                ));
            }
        }
        let mut series = self.y.len();
        if !self.color.is_empty() {
            let Some(&idx) = cols.get(&self.color) else {
                return Err(ChartError::new(
                    CODE_COLUMN_UNKNOWN,
                    &format!("{path}/color"),
                ));
            };
            if data.columns[idx].column_type != "string" {
                return Err(ChartError::new(
                    CODE_TYPE_MISMATCH,
                    &format!("{path}/color"),
                ));
            }
            series *= distinct(data, idx);
        }
        if series > MAX_SERIES {
            return Err(ChartError::exceeded(CAP_SERIES, path));
        }
        if data.rows.len() * self.y.len() > MAX_POINTS {
            return Err(ChartError::exceeded(CAP_POINTS, path));
        }
        self.check_values(path, cols, data, y_axis)
    }

    fn check_values(
        &self,
        path: &str,
        cols: &std::collections::HashMap<String, usize>,
        data: &Data,
        y_axis: Option<&Axis>,
    ) -> Result<(), ChartError> {
        let log_scale = y_axis.is_some_and(|a| a.scale == "log");
        if self.kind == "pie" && log_scale {
            return Err(ChartError::new(CODE_SCHEMA_INVALID, "/y_axis/scale"));
        }
        for y in &self.y {
            let &yi = cols.get(y).expect("y column validated above");
            for (r, row) in data.rows.iter().enumerate() {
                let cell = &row[yi];
                if cell.kind != CellKind::Number {
                    continue;
                }
                let p = format!("/data/rows/{r}/{yi}");
                if (self.kind == "pie" && cell.num < 0.0) || (log_scale && cell.num <= 0.0) {
                    return Err(ChartError::new(CODE_SCHEMA_INVALID, &p));
                }
            }
        }
        let _ = path;
        Ok(())
    }
}

fn distinct(data: &Data, col: usize) -> usize {
    let mut seen = std::collections::HashSet::new();
    for row in &data.rows {
        if row[col].kind == CellKind::String {
            seen.insert(row[col].str.clone());
        }
    }
    seen.len()
}

fn check_label(s: &str, path: &str) -> Result<(), ChartError> {
    if s.chars().count() > MAX_LABEL_RUNES {
        return Err(ChartError::exceeded(CAP_LABEL, path));
    }
    if !std::str::from_utf8(s.as_bytes()).is_ok() {
        return Err(ChartError::new(CODE_SCHEMA_INVALID, path));
    }
    if s.chars().any(|r| r.is_control()) {
        return Err(ChartError::new(CODE_SCHEMA_INVALID, path));
    }
    Ok(())
}

// --- summary -----------------------------------------------------------------

const QUOTED_RUNES: usize = 32;

impl Spec {
    /// The only view of a chart the model reads back: names, counts and numeric
    /// extremes, every cell string quoted and truncated (Go `Spec.Summary`).
    pub fn summary(&self) -> String {
        let mut b = String::new();
        b.push_str(&format!(
            "chart {}, {} rows, {} columns",
            quote(&self.title),
            self.data.rows.len(),
            self.data.columns.len()
        ));
        write_axis(&mut b, "x_axis", self.x_axis.as_ref());
        write_axis(&mut b, "y_axis", self.y_axis.as_ref());
        let mut idx = std::collections::HashMap::new();
        for (i, c) in self.data.columns.iter().enumerate() {
            idx.insert(c.name.clone(), i);
        }
        for mark in &self.marks {
            b.push_str(&format!("\n{} x={}", mark.kind, quote(&mark.x)));
            if mark.stacked {
                b.push_str(" stacked");
            }
            if mark.donut {
                b.push_str(" donut");
            }
            for y in &mark.y {
                series_lines(&mut b, mark, y, &idx, &self.data);
            }
        }
        b
    }
}

fn series_lines(
    b: &mut String,
    mark: &Mark,
    y: &str,
    idx: &std::collections::HashMap<String, usize>,
    data: &Data,
) {
    let yi = *idx.get(y).expect("y column validated above");
    if mark.color.is_empty() {
        write_extremes(b, &quote(y), &data.rows, yi, usize::MAX, "");
        return;
    }
    let ci = *idx.get(&mark.color).expect("color column validated above");
    let mut order: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in &data.rows {
        if row[ci].kind == CellKind::String && !seen.contains(&row[ci].str) {
            seen.insert(row[ci].str.clone());
            order.push(row[ci].str.clone());
        }
    }
    for v in &order {
        write_extremes(
            b,
            &format!("{}/{}", quote(y), quote(v)),
            &data.rows,
            yi,
            ci,
            v,
        );
    }
}

fn write_extremes(
    b: &mut String,
    name: &str,
    rows: &[Vec<Cell>],
    yi: usize,
    ci: usize,
    only: &str,
) {
    let mut n = 0usize;
    let mut nulls = 0usize;
    let mut lo = 0.0;
    let mut hi = 0.0;
    for row in rows {
        if ci != usize::MAX && (row[ci].kind != CellKind::String || row[ci].str != only) {
            continue;
        }
        let cell = &row[yi];
        if cell.kind != CellKind::Number {
            nulls += 1;
            continue;
        }
        if n == 0 || cell.num < lo {
            lo = cell.num;
        }
        if n == 0 || cell.num > hi {
            hi = cell.num;
        }
        n += 1;
    }
    b.push_str(&format!("\n  series {name} n={n}"));
    if n > 0 {
        b.push_str(&format!(" min={} max={}", num(lo), num(hi)));
    }
    if nulls > 0 {
        b.push_str(&format!(" null={nulls}"));
    }
}

fn num(f: f64) -> String {
    format!("{f}")
}

fn quote(s: &str) -> String {
    let runes: Vec<char> = s.chars().collect();
    if runes.len() > QUOTED_RUNES {
        let truncated: String = runes[..QUOTED_RUNES].iter().collect();
        return format!("{:?}...", truncated);
    }
    format!("{s:?}")
}

fn write_axis(b: &mut String, name: &str, axis: Option<&Axis>) {
    let Some(axis) = axis else { return };
    b.push_str(&format!(
        "\n{name} title={} unit={}",
        quote(&axis.title),
        quote(&axis.unit)
    ));
    if !axis.scale.is_empty() {
        b.push_str(&format!(" scale={}", axis.scale));
    }
}

/// The chart id: `"chart-"` + first six bytes of the canonical spec's SHA-256
/// (Go `ChartID`). The hash comes from the workspace `sha2` crate.
pub fn chart_id(spec: &Spec) -> Result<String, ChartError> {
    let canonical = spec.canonical()?;
    let sum = Sha256::digest(canonical);
    Ok(format!("chart-{}", hex_lower(&sum[..6])))
}

/// Bytes → lowercase hex.
fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_spec() -> Spec {
        parse(
            r#"{
                "spec_version": 1,
                "title": "Sales",
                "data": {
                    "columns": [
                        {"name": "region", "type": "string"},
                        {"name": "units", "type": "number"}
                    ],
                    "rows": [["East", 10], ["West", 20]]
                },
                "marks": [{"type": "bar", "x": "region", "y": ["units"]}]
            }"#
            .as_bytes(),
        )
        .expect("valid spec parses")
    }

    #[test]
    fn parses_valid_spec_and_summary_names_counts() {
        let spec = valid_spec();
        let summary = spec.summary();
        assert!(summary.starts_with("chart \"Sales\", 2 rows, 2 columns"));
        assert!(summary.contains("bar x=\"region\""));
        assert!(summary.contains("series \"units\" n=2 min=10 max=20"));
    }

    #[test]
    fn chart_id_is_stable_and_prefixed() {
        let spec = valid_spec();
        let id = chart_id(&spec).expect("chart id");
        assert!(id.starts_with("chart-"));
        assert_eq!(id.len(), "chart-".len() + 12);
        assert_eq!(id, chart_id(&spec).expect("same spec same id"));
    }

    #[test]
    fn refuses_unsupported_version() {
        let err = parse(
            r#"{"spec_version":2,"title":"t","data":{"columns":[],"rows":[]},"marks":[]}"#
                .as_bytes(),
        )
        .expect_err("version 2 refused");
        assert_eq!(err.code, CODE_SPEC_VERSION_UNSUPPORTED);
    }

    #[test]
    fn refuses_unknown_column_in_mark() {
        let err = parse(
            r#"{"spec_version":1,"title":"t","data":{"columns":[{"name":"a","type":"string"}],"rows":[]},"marks":[{"type":"bar","x":"nope","y":[]}]}"#
                .as_bytes(),
        )
        .expect_err("unknown column refused");
        assert_eq!(err.code, CODE_COLUMN_UNKNOWN);
    }

    #[test]
    fn refuses_duplicate_keys() {
        let err = parse(
            r#"{"spec_version":1,"spec_version":1,"title":"t","data":{"columns":[],"rows":[]},"marks":[]}"#
                .as_bytes(),
        )
        .expect_err("duplicate key refused");
        assert_eq!(err.code, CODE_SCHEMA_INVALID);
    }

    #[test]
    fn type_mismatch_number_vs_string() {
        let err = parse(
            r#"{"spec_version":1,"title":"t","data":{"columns":[{"name":"n","type":"number"}],"rows":[["ten"]]},"marks":[]}"#
                .as_bytes(),
        )
        .expect_err("mismatch refused");
        assert_eq!(err.code, CODE_TYPE_MISMATCH);
    }
}
