//! Typed California Dashboard response models and fetch provenance.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Immutable mapping published by the California School Dashboard.
pub const INDICATOR_CATEGORIES: [(u8, &str); 8] = [
    (1, "CHRONIC_ABSENTEEISM"),
    (2, "SUSPENSION_RATE"),
    (3, "ENGLISH_LEARNER_PROGRESS"),
    (4, "GRADUATION_RATE"),
    (5, "COLLEGE_CAREER_INDICATOR"),
    (6, "ELA_POINTS_ABOVE_BELOW"),
    (7, "MATHEMATICS"),
    (8, "SCIENCE"),
];

pub const fn indicator_category(indicator_id: u8) -> Option<&'static str> {
    match indicator_id {
        1 => Some("CHRONIC_ABSENTEEISM"),
        2 => Some("SUSPENSION_RATE"),
        3 => Some("ENGLISH_LEARNER_PROGRESS"),
        4 => Some("GRADUATION_RATE"),
        5 => Some("COLLEGE_CAREER_INDICATOR"),
        6 => Some("ELA_POINTS_ABOVE_BELOW"),
        7 => Some("MATHEMATICS"),
        8 => Some("SCIENCE"),
        _ => None,
    }
}

/// An active public school loaded from the CDE SQLite cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchoolRecord {
    pub cds_code: String,
    pub school_name: String,
    pub county: Option<String>,
    pub district: Option<String>,
    pub city: Option<String>,
}

/// One row in the CDE-style school and district directory export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryRecord {
    pub record_type: String,
    pub cds_code: String,
    pub district: String,
    pub school: String,
    pub status: String,
}

/// A typed primary or comparison measure from an indicator response.
///
/// Missing and privacy-suppressed values remain `None`; the exact JSON blocks
/// are retained separately on [`Indicator`] for lossless C++ parity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Measure {
    pub cds_code: Option<String>,
    pub indicator_id: Option<u8>,
    #[serde(default, deserialize_with = "deserialize_optional_f64")]
    pub status: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_optional_f64")]
    pub change: Option<f64>,
    pub change_id: Option<i32>,
    pub status_id: Option<i32>,
    pub performance: Option<i32>,
    pub total_groups: Option<u32>,
    pub red: Option<u32>,
    pub orange: Option<u32>,
    pub yellow: Option<u32>,
    pub green: Option<u32>,
    pub blue: Option<u32>,
    pub count: Option<u64>,
    pub student_group: Option<String>,
    pub school_year_id: Option<u8>,
    pub is_private_data: Option<bool>,
}

/// One Dashboard indicator and its school and statewide/comparison measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Indicator {
    pub indicator_id: u8,
    pub category: String,
    #[serde(default)]
    pub primary_json: Value,
    #[serde(default)]
    pub secondary_json: Value,
    pub primary: Option<Measure>,
    pub secondary: Option<Measure>,
}

/// A single, fully identified network request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchSpec {
    pub school: SchoolRecord,
    pub year: u16,
    pub dashboard_year_id: u8,
    pub url: String,
}

/// Reproducibility metadata attached to every fetch result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_url: String,
    pub retrieved_at_utc: String,
    pub payload_sha256: Option<String>,
    pub payload_bytes: usize,
    pub http_status: Option<u16>,
    pub attempts: u8,
    pub method_version: String,
}

/// A parsed summary card with exact raw JSON and typed fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryCard {
    pub school: SchoolRecord,
    pub year: u16,
    pub dashboard_year_id: u8,
    pub indicators: Vec<Indicator>,
    pub provenance: Provenance,
    #[serde(default)]
    pub raw_data: String,
    #[serde(default)]
    pub raw_json_data: Value,
}

impl SummaryCard {
    /// Parses either the API's usual top-level array or a singleton value.
    pub fn from_payload(
        spec: &FetchSpec,
        payload: &[u8],
        provenance: Provenance,
    ) -> Result<Self, ModelError> {
        let raw_data = String::from_utf8_lossy(payload).into_owned();
        let raw_json_data: Value = serde_json::from_slice(payload)?;
        Ok(Self {
            school: spec.school.clone(),
            year: spec.year,
            dashboard_year_id: spec.dashboard_year_id,
            indicators: parse_indicators_value(&raw_json_data),
            provenance,
            raw_data,
            raw_json_data,
        })
    }

    /// Mirrors the C++ constructor that accepts a raw JSON string.
    pub fn from_json_str(json: &str) -> Result<Self, ModelError> {
        let raw_json_data: Value = serde_json::from_str(json)?;
        Ok(Self {
            school: SchoolRecord {
                cds_code: String::new(),
                school_name: String::new(),
                county: None,
                district: None,
                city: None,
            },
            year: 0,
            dashboard_year_id: 0,
            indicators: parse_indicators_value(&raw_json_data),
            provenance: Provenance {
                source_url: String::new(),
                retrieved_at_utc: String::new(),
                payload_sha256: None,
                payload_bytes: json.len(),
                http_status: None,
                attempts: 0,
                method_version: String::new(),
            },
            raw_data: json.to_owned(),
            raw_json_data,
        })
    }

    pub fn set_raw_data(&mut self, data: impl Into<String>) {
        self.raw_data = data.into();
    }

    pub fn append_raw_data(&mut self, data: &[u8]) {
        self.raw_data.push_str(&String::from_utf8_lossy(data));
    }

    pub fn parse_raw_data(&mut self) -> Result<(), ModelError> {
        if self.raw_data.is_empty() {
            self.raw_json_data = Value::Array(Vec::new());
            self.indicators.clear();
            return Ok(());
        }
        let value: Value = serde_json::from_str(&self.raw_data)?;
        self.indicators = parse_indicators_value(&value);
        self.raw_json_data = value;
        self.provenance.payload_bytes = self.raw_data.len();
        Ok(())
    }

    pub fn clear(&mut self) {
        self.raw_data.clear();
        self.raw_json_data = Value::Array(Vec::new());
        self.indicators.clear();
        self.school.cds_code.clear();
        self.school.school_name.clear();
        self.school.county = None;
        self.school.district = None;
        self.school.city = None;
        self.year = 0;
        self.dashboard_year_id = 0;
        self.provenance.source_url.clear();
        self.provenance.retrieved_at_utc.clear();
        self.provenance.payload_sha256 = None;
        self.provenance.payload_bytes = 0;
        self.provenance.http_status = None;
        self.provenance.attempts = 0;
        self.provenance.method_version.clear();
    }

    pub fn set_metadata(&mut self, school: impl Into<String>, year: impl AsRef<str>) {
        self.school.school_name = school.into();
        self.year = year.as_ref().trim().parse().unwrap_or_default();
        self.dashboard_year_id = crate::years::dashboard_year_id(self.year).unwrap_or_default();
    }

    pub fn raw_data(&self) -> &str {
        &self.raw_data
    }

    pub const fn raw_json_data(&self) -> &Value {
        &self.raw_json_data
    }

    pub fn indicator_vector(&self) -> Vec<Indicator> {
        self.indicators.clone()
    }

    /// Last value wins for duplicate categories, matching the C++ map.
    pub fn category_map(&self) -> BTreeMap<String, Indicator> {
        self.indicators
            .iter()
            .cloned()
            .map(|indicator| (indicator.category.clone(), indicator))
            .collect()
    }

    pub fn print_raw_data(&self) -> bool {
        if self.raw_data.is_empty() {
            return false;
        }
        println!("{}", self.raw_data);
        true
    }

    pub fn print_indicator_vector(&self) -> bool {
        if self.indicators.is_empty() {
            return false;
        }
        let mut output = io::BufWriter::new(io::stdout().lock());
        if !self.school.school_name.is_empty() || self.year != 0 {
            let year = if self.year == 0 {
                "Unknown".to_owned()
            } else {
                self.year.to_string()
            };
            let school = if self.school.school_name.is_empty() {
                "Unknown"
            } else {
                &self.school.school_name
            };
            let _ = writeln!(
                output,
                "=============================\nSchool: {school}\nYear:   {year}\n============================="
            );
        }
        for indicator in &self.indicators {
            let primary = indicator.primary.as_ref();
            let _ = writeln!(
                output,
                "-----------------------------\nCategory:     {}\nCDS Code:     {}\nIndicator ID: {}\nStatus:       {}\nChange:       {}\nStatus ID:    {}\nPerformance:  {}\nTotal Groups: {}\nCount:        {}\nStudent Group:{}\nColors:       R={} O={} Y={} G={} B={}\nPrivate Data: {}",
                indicator.category,
                primary
                    .and_then(|measure| measure.cds_code.as_deref())
                    .unwrap_or(""),
                indicator.indicator_id,
                option_number(primary.and_then(|measure| measure.status)),
                option_number(primary.and_then(|measure| measure.change)),
                option_number(primary.and_then(|measure| measure.status_id)),
                option_number(primary.and_then(|measure| measure.performance)),
                option_number(primary.and_then(|measure| measure.total_groups)),
                option_number(primary.and_then(|measure| measure.count)),
                primary
                    .and_then(|measure| measure.student_group.as_deref())
                    .unwrap_or(""),
                option_number(primary.and_then(|measure| measure.red)),
                option_number(primary.and_then(|measure| measure.orange)),
                option_number(primary.and_then(|measure| measure.yellow)),
                option_number(primary.and_then(|measure| measure.green)),
                option_number(primary.and_then(|measure| measure.blue)),
                primary
                    .and_then(|measure| measure.is_private_data)
                    .unwrap_or(false),
            );
        }
        output.flush().is_ok()
    }

    pub fn save_to_file(&self, filename: impl AsRef<Path>) -> Result<(), ModelError> {
        let file = std::fs::File::create(filename)?;
        serde_json::to_writer(file, &self.raw_json_data)?;
        Ok(())
    }

    pub fn load_from_file(&mut self, filename: impl AsRef<Path>) -> Result<(), ModelError> {
        self.raw_data = std::fs::read_to_string(filename)?;
        self.parse_raw_data()
    }
}

fn option_number<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "0".to_owned(), |number| number.to_string())
}

/// The terminal state of one request. Failures are data, not dropped rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FetchStatus {
    Success,
    HttpError {
        status_code: u16,
    },
    TransportError {
        message: String,
    },
    PayloadTooLarge {
        limit_bytes: usize,
        declared_bytes: Option<u64>,
    },
    EmptyPayload,
    InvalidJson {
        message: String,
    },
}

impl FetchStatus {
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FetchOutcome {
    pub spec: FetchSpec,
    pub status: FetchStatus,
    pub card: Option<SummaryCard>,
    pub provenance: Provenance,
}

impl FetchOutcome {
    pub const fn is_success(&self) -> bool {
        self.status.is_success()
    }
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("dashboard response is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("summary-card file operation failed: {0}")]
    Io(#[from] io::Error),
}

/// Parses indicators tolerantly: non-object entries are skipped and missing
/// fields affect only that field, matching the original C++ parser.
pub fn parse_indicators(payload: &[u8]) -> Result<Vec<Indicator>, ModelError> {
    let value: Value = serde_json::from_slice(payload)?;
    Ok(parse_indicators_value(&value))
}

fn parse_indicators_value(value: &Value) -> Vec<Indicator> {
    let entries: Vec<&Value> = match value {
        Value::Null => Vec::new(),
        Value::Array(entries) => entries.iter().collect(),
        singleton => vec![singleton],
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let object = entry.as_object()?;
            let indicator_id = object
                .get("indicatorId")
                .and_then(value_u64)
                .and_then(|number| u8::try_from(number).ok())
                .unwrap_or_default();
            let primary_json = object.get("primary").cloned().unwrap_or(Value::Null);
            let secondary_json = object.get("secondary").cloned().unwrap_or(Value::Null);
            Some(Indicator {
                indicator_id,
                category: indicator_category(indicator_id)
                    .unwrap_or("UNKNOWN")
                    .to_owned(),
                primary: primary_json.as_object().map(parse_measure),
                secondary: secondary_json.as_object().map(parse_measure),
                primary_json,
                secondary_json,
            })
        })
        .collect()
}

fn parse_measure(object: &serde_json::Map<String, Value>) -> Measure {
    Measure {
        cds_code: object.get("cdsCode").and_then(value_string),
        indicator_id: object
            .get("indicatorId")
            .and_then(value_u64)
            .and_then(|number| u8::try_from(number).ok()),
        status: object.get("status").and_then(value_f64),
        change: object.get("change").and_then(value_f64),
        change_id: object.get("changeId").and_then(value_i32),
        status_id: object.get("statusId").and_then(value_i32),
        performance: object.get("performance").and_then(value_i32),
        total_groups: object
            .get("totalGroups")
            .and_then(value_u64)
            .and_then(|number| u32::try_from(number).ok()),
        red: object
            .get("red")
            .and_then(value_u64)
            .and_then(|number| u32::try_from(number).ok()),
        orange: object
            .get("orange")
            .and_then(value_u64)
            .and_then(|number| u32::try_from(number).ok()),
        yellow: object
            .get("yellow")
            .and_then(value_u64)
            .and_then(|number| u32::try_from(number).ok()),
        green: object
            .get("green")
            .and_then(value_u64)
            .and_then(|number| u32::try_from(number).ok()),
        blue: object
            .get("blue")
            .and_then(value_u64)
            .and_then(|number| u32::try_from(number).ok()),
        count: object.get("count").and_then(value_u64),
        student_group: object.get("studentGroup").and_then(value_string),
        school_year_id: object
            .get("schoolYearId")
            .and_then(value_u64)
            .and_then(|number| u8::try_from(number).ok()),
        is_private_data: object.get("isPrivateData").and_then(Value::as_bool),
    }
}

fn value_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

fn value_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .filter(|number| number.is_finite())
}

fn value_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
}

fn value_i32(value: &Value) -> Option<i32> {
    value.as_i64().and_then(|number| i32::try_from(number).ok())
}

fn deserialize_optional_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_f64()
            .ok_or_else(|| serde::de::Error::custom("numeric value is outside f64 range"))
            .map(Some),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(Value::String(text)) => text
            .trim()
            .parse::<f64>()
            .map(Some)
            .map_err(serde::de::Error::custom),
        Some(other) => Err(serde::de::Error::custom(format!(
            "expected a number, numeric string, or null; got {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_skips_non_objects_and_retains_raw_blocks() {
        let parsed = parse_indicators(
            br#"[null,7,{"primary":{"status":"not numeric","cdsCode":42}},{"indicatorId":8,"primary":{"status":12.5},"secondary":{"extra":true}}]"#,
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].indicator_id, 0);
        assert_eq!(parsed[0].category, "UNKNOWN");
        assert_eq!(parsed[0].primary.as_ref().unwrap().status, None);
        assert_eq!(
            parsed[0].primary.as_ref().unwrap().cds_code.as_deref(),
            Some("42")
        );
        assert_eq!(parsed[1].primary_json["status"], 12.5);
        assert_eq!(parsed[1].secondary_json["extra"], true);
    }

    #[test]
    fn raw_file_round_trip_matches_cpp_surface() {
        let mut card =
            SummaryCard::from_json_str(r#"{"indicatorId":2,"primary":{"status":1.5}}"#).unwrap();
        card.set_metadata("Example School", "2024");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("card.json");
        card.save_to_file(&path).unwrap();

        card.clear();
        assert!(card.indicators.is_empty());
        card.load_from_file(path).unwrap();
        assert_eq!(card.indicators.len(), 1);
        assert_eq!(card.indicators[0].indicator_id, 2);
        assert_eq!(card.raw_json_data()["primary"]["status"], 1.5);
    }
}
