//! Privacy-preserving descriptive statistics used by every presentation layer.
//!
//! This module deliberately creates one canonical set of rows.  The web UI and
//! every export consume those rows instead of independently interpreting the
//! Dashboard payload.  In particular, a private value or a value with a known
//! denominator of ten or fewer never enters the canonical data set.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::METHOD_VERSION;
use crate::model::{FetchOutcome, FetchStatus, Indicator, Measure, SummaryCard};

const NARRATIVE_ROW_LIMIT: usize = 200;

/// Why a primary or comparison value is unavailable in the canonical data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MissingReason {
    MissingPrimary,
    MissingComparator,
    MissingStatus,
    PrivateData,
    SmallN,
}

impl MissingReason {
    /// Stable, machine-readable code used in all exports.
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingPrimary => "MISSING_PRIMARY",
            Self::MissingComparator => "MISSING_COMPARATOR",
            Self::MissingStatus => "MISSING_STATUS",
            Self::PrivateData => "PRIVATE_DATA",
            Self::SmallN => "SMALL_N",
        }
    }

    pub const fn explanation(self) -> &'static str {
        match self {
            Self::MissingPrimary => "The API did not return a primary measure.",
            Self::MissingComparator => "The API did not return a comparison measure.",
            Self::MissingStatus => "The measure did not contain a finite status value.",
            Self::PrivateData => "The API marked the measure as private.",
            Self::SmallN => "The known denominator was ten or fewer.",
        }
    }
}

impl fmt::Display for MissingReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// The direction in which an indicator is conventionally favorable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FavorableDirection {
    Higher,
    Lower,
}

impl FavorableDirection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Higher => "higher is favorable",
            Self::Lower => "lower is favorable",
        }
    }

    const fn multiplier(self) -> f64 {
        match self {
            Self::Higher => 1.0,
            Self::Lower => -1.0,
        }
    }
}

/// Auditable interpretation metadata for a Dashboard indicator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndicatorDefinition {
    pub indicator_id: u8,
    pub category: String,
    pub display_name: String,
    pub status_unit: String,
    pub change_unit: String,
    pub favorable_direction: FavorableDirection,
    pub description: String,
}

/// A privacy-filtered school/year/indicator observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalRow {
    pub trace_id: String,
    pub school_name: String,
    pub cds_code: String,
    pub year: u16,
    pub dashboard_year_id: u8,
    pub indicator_id: u8,
    pub category: String,
    pub indicator_name: String,
    pub status_unit: String,
    pub change_unit: String,
    pub favorable_direction: FavorableDirection,

    /// School status, removed when privacy or small-denominator rules apply.
    pub status: Option<f64>,
    /// The Dashboard's supplied change value. It is never recomputed.
    pub official_change: Option<f64>,
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

    /// Public comparison status, subject to the same privacy rules.
    pub comparator_status: Option<f64>,
    pub comparator_count: Option<u64>,
    /// School status minus comparison status.
    pub raw_comparator_gap: Option<f64>,
    /// Raw gap multiplied by the indicator's favorable-direction sign.
    pub favorable_comparator_gap: Option<f64>,

    pub missing_reason: Option<MissingReason>,
    pub comparator_missing_reason: Option<MissingReason>,
    /// True only for a published value with a known denominator from 11 to 29.
    pub small_n_warning: bool,
    pub comparator_small_n_warning: bool,
    pub year_caveat: Option<String>,
    pub informational_only: bool,

    pub source_url: String,
    pub retrieved_at_utc: String,
    pub payload_sha256: Option<String>,
    pub method_version: String,
}

impl CanonicalRow {
    pub const fn is_reported(&self) -> bool {
        self.status.is_some()
    }

    pub fn missing_reason_code(&self) -> &str {
        self.missing_reason.map_or("", MissingReason::code)
    }

    pub fn comparator_missing_reason_code(&self) -> &str {
        self.comparator_missing_reason
            .map_or("", MissingReason::code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DataQuality {
    pub fetches_total: usize,
    pub fetches_succeeded: usize,
    pub fetches_failed: usize,
    pub cards_total: usize,
    pub rows_total: usize,
    pub rows_reported: usize,
    pub rows_missing: usize,
    pub rows_private: usize,
    pub rows_small_n_suppressed: usize,
    pub rows_small_n_warning: usize,
    pub comparators_missing_or_suppressed: usize,
    pub duplicate_indicators: usize,
    pub unexpected_indicators: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchLogEntry {
    pub school_name: String,
    pub cds_code: String,
    pub year: u16,
    pub dashboard_year_id: u8,
    pub status: String,
    pub detail: String,
    pub source_url: String,
    pub retrieved_at_utc: String,
    pub payload_sha256: Option<String>,
    pub payload_bytes: usize,
    pub http_status: Option<u16>,
    pub attempts: u8,
    pub method_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NarrativeStatement {
    pub trace_id: String,
    pub text: String,
}

/// The sole statistical/reporting model used by the UI and all exports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportModel {
    pub method_version: String,
    pub rows: Vec<CanonicalRow>,
    pub definitions: Vec<IndicatorDefinition>,
    pub fetch_log: Vec<FetchLogEntry>,
    pub quality: DataQuality,
    pub narrative: Vec<NarrativeStatement>,
    pub methods: Vec<String>,
    pub sources: Vec<String>,
}

impl ReportModel {
    /// Builds a canonical report from successfully parsed cards.
    pub fn from_cards(cards: &[SummaryCard]) -> Self {
        let definitions = indicator_definitions();
        let mut sorted_cards: Vec<&SummaryCard> = cards.iter().collect();
        sorted_cards.sort_by(|left, right| {
            (
                left.school.cds_code.as_str(),
                left.year,
                left.dashboard_year_id,
            )
                .cmp(&(
                    right.school.cds_code.as_str(),
                    right.year,
                    right.dashboard_year_id,
                ))
        });

        let mut rows = Vec::with_capacity(cards.len().saturating_mul(definitions.len()));
        let mut duplicate_indicators = 0usize;
        let mut unexpected_indicators = 0usize;

        for card in sorted_cards {
            let mut by_id: BTreeMap<u8, &Indicator> = BTreeMap::new();
            for indicator in &card.indicators {
                if !(1..=8).contains(&indicator.indicator_id) {
                    unexpected_indicators += 1;
                    continue;
                }
                if by_id.insert(indicator.indicator_id, indicator).is_some() {
                    duplicate_indicators += 1;
                }
            }

            for definition in &definitions {
                rows.push(canonicalize_row(
                    card,
                    definition,
                    by_id.get(&definition.indicator_id).copied(),
                ));
            }
        }

        let fetch_log = cards.iter().map(fetch_log_from_card).collect::<Vec<_>>();
        let mut quality = quality_from_rows(&rows);
        quality.fetches_total = cards.len();
        quality.fetches_succeeded = cards.len();
        quality.cards_total = cards.len();
        quality.duplicate_indicators = duplicate_indicators;
        quality.unexpected_indicators = unexpected_indicators;
        let narrative = narrative_from_rows(&rows);

        Self {
            method_version: METHOD_VERSION.to_owned(),
            rows,
            definitions,
            fetch_log,
            quality,
            narrative,
            methods: methods_text(),
            sources: sources_text(),
        }
    }

    /// Builds a report from every completed request, preserving failed fetches
    /// in the audit log while canonicalizing only successful cards.
    pub fn from_outcomes(outcomes: &[FetchOutcome]) -> Self {
        let cards = outcomes
            .iter()
            .filter_map(|outcome| outcome.card.clone())
            .collect::<Vec<_>>();
        let mut report = Self::from_cards(&cards);
        report.fetch_log = outcomes.iter().map(fetch_log_from_outcome).collect();
        report.fetch_log.sort_by(|left, right| {
            (left.cds_code.as_str(), left.year, left.source_url.as_str()).cmp(&(
                right.cds_code.as_str(),
                right.year,
                right.source_url.as_str(),
            ))
        });
        report.quality.fetches_total = outcomes.len();
        report.quality.fetches_succeeded = outcomes
            .iter()
            .filter(|outcome| outcome.status.is_success() && outcome.card.is_some())
            .count();
        report.quality.fetches_failed = report
            .quality
            .fetches_total
            .saturating_sub(report.quality.fetches_succeeded);
        report
    }

    pub fn rows(&self) -> &[CanonicalRow] {
        &self.rows
    }

    pub fn quality(&self) -> &DataQuality {
        &self.quality
    }

    pub fn narrative(&self) -> &[NarrativeStatement] {
        &self.narrative
    }
}

/// Returns definitions for all eight indicators in stable ID order.
pub fn indicator_definitions() -> Vec<IndicatorDefinition> {
    vec![
        definition(
            1,
            "CHRONIC_ABSENTEEISM",
            "Chronic Absenteeism",
            "percent",
            "percentage points",
            FavorableDirection::Lower,
            "Students absent for 10 percent or more of instructional days.",
        ),
        definition(
            2,
            "SUSPENSION_RATE",
            "Suspension Rate",
            "percent",
            "percentage points",
            FavorableDirection::Lower,
            "Students suspended at least once during the academic year.",
        ),
        definition(
            3,
            "ENGLISH_LEARNER_PROGRESS",
            "English Learner Progress",
            "percent",
            "percentage points",
            FavorableDirection::Higher,
            "English learners making progress toward English-language proficiency.",
        ),
        definition(
            4,
            "GRADUATION_RATE",
            "Graduation Rate",
            "percent",
            "percentage points",
            FavorableDirection::Higher,
            "Students completing high school within the Dashboard graduation-rate definition.",
        ),
        definition(
            5,
            "COLLEGE_CAREER_INDICATOR",
            "College/Career Indicator",
            "percent prepared",
            "percentage points",
            FavorableDirection::Higher,
            "Graduates meeting the Dashboard prepared criteria for college or career.",
        ),
        definition(
            6,
            "ELA_POINTS_ABOVE_BELOW",
            "English Language Arts",
            "points from standard",
            "points",
            FavorableDirection::Higher,
            "Average distance above or below the English language arts standard.",
        ),
        definition(
            7,
            "MATHEMATICS",
            "Mathematics",
            "points from standard",
            "points",
            FavorableDirection::Higher,
            "Average distance above or below the mathematics standard.",
        ),
        definition(
            8,
            "SCIENCE",
            "Science",
            "science points (0–100)",
            "science points",
            FavorableDirection::Higher,
            "Average California Science Test performance expressed on the Dashboard's 0–100 Science Points scale.",
        ),
    ]
}

fn definition(
    indicator_id: u8,
    category: &str,
    display_name: &str,
    status_unit: &str,
    change_unit: &str,
    favorable_direction: FavorableDirection,
    description: &str,
) -> IndicatorDefinition {
    IndicatorDefinition {
        indicator_id,
        category: category.to_owned(),
        display_name: display_name.to_owned(),
        status_unit: status_unit.to_owned(),
        change_unit: change_unit.to_owned(),
        favorable_direction,
        description: description.to_owned(),
    }
}

fn canonicalize_row(
    card: &SummaryCard,
    definition: &IndicatorDefinition,
    indicator: Option<&Indicator>,
) -> CanonicalRow {
    let primary = indicator.and_then(|value| value.primary.as_ref());
    let comparator = indicator.and_then(|value| value.secondary.as_ref());
    let primary_reason = measure_reason(primary, false);
    let comparator_reason = measure_reason(comparator, true);
    let primary_visible = primary_reason.is_none();
    let comparator_visible = comparator_reason.is_none();
    let primary = primary.filter(|_| primary_visible);
    let comparator = comparator.filter(|_| comparator_visible);

    let status = primary.and_then(|measure| finite(measure.status));
    let comparator_status = comparator.and_then(|measure| finite(measure.status));
    let raw_comparator_gap = status
        .zip(comparator_status)
        .map(|(school, comparison)| school - comparison)
        .filter(|value| value.is_finite());
    let favorable_comparator_gap = raw_comparator_gap
        .map(|gap| gap * definition.favorable_direction.multiplier())
        .filter(|value| value.is_finite());
    let (year_caveat, informational_only) = year_caveat(card.year, definition.indicator_id);
    let classification_allowed = classification_is_official(card.year, definition.indicator_id);
    let change_allowed = card.year != 2022;
    let color_counts_allowed = !matches!(card.year, 2020..=2022);

    CanonicalRow {
        trace_id: format!(
            "{}/{}/I{}",
            card.school.cds_code, card.year, definition.indicator_id
        ),
        school_name: card.school.school_name.clone(),
        cds_code: card.school.cds_code.clone(),
        year: card.year,
        dashboard_year_id: card.dashboard_year_id,
        indicator_id: definition.indicator_id,
        category: definition.category.clone(),
        indicator_name: definition.display_name.clone(),
        status_unit: definition.status_unit.clone(),
        change_unit: definition.change_unit.clone(),
        favorable_direction: definition.favorable_direction,
        status,
        official_change: primary
            .and_then(|measure| finite(measure.change))
            .filter(|_| change_allowed),
        change_id: primary
            .and_then(|measure| measure.change_id)
            .filter(|_| change_allowed),
        status_id: primary.and_then(|measure| measure.status_id),
        // Policy: only a nonzero official performance level is displayed. A zero
        // (or null) means "no performance color" and must not surface as a level.
        performance: primary
            .and_then(|measure| measure.performance)
            .filter(|value| *value != 0)
            .filter(|_| classification_allowed),
        total_groups: primary.and_then(|measure| measure.total_groups),
        red: primary
            .and_then(|measure| measure.red)
            .filter(|_| color_counts_allowed),
        orange: primary
            .and_then(|measure| measure.orange)
            .filter(|_| color_counts_allowed),
        yellow: primary
            .and_then(|measure| measure.yellow)
            .filter(|_| color_counts_allowed),
        green: primary
            .and_then(|measure| measure.green)
            .filter(|_| color_counts_allowed),
        blue: primary
            .and_then(|measure| measure.blue)
            .filter(|_| color_counts_allowed),
        count: primary.and_then(|measure| measure.count),
        student_group: primary.and_then(|measure| measure.student_group.clone()),
        comparator_status,
        comparator_count: comparator.and_then(|measure| measure.count),
        raw_comparator_gap,
        favorable_comparator_gap,
        missing_reason: primary_reason,
        comparator_missing_reason: comparator_reason,
        small_n_warning: primary
            .and_then(|measure| measure.count)
            .is_some_and(|count| (11..=29).contains(&count)),
        comparator_small_n_warning: comparator
            .and_then(|measure| measure.count)
            .is_some_and(|count| (11..=29).contains(&count)),
        year_caveat,
        informational_only,
        source_url: card.provenance.source_url.clone(),
        retrieved_at_utc: card.provenance.retrieved_at_utc.clone(),
        payload_sha256: card.provenance.payload_sha256.clone(),
        method_version: METHOD_VERSION.to_owned(),
    }
}

fn measure_reason(measure: Option<&Measure>, comparator: bool) -> Option<MissingReason> {
    let Some(measure) = measure else {
        return Some(if comparator {
            MissingReason::MissingComparator
        } else {
            MissingReason::MissingPrimary
        });
    };
    if measure.is_private_data == Some(true) {
        return Some(MissingReason::PrivateData);
    }
    if measure.count.is_some_and(|count| count <= 10) {
        return Some(MissingReason::SmallN);
    }
    if finite(measure.status).is_none() {
        return Some(MissingReason::MissingStatus);
    }
    None
}

fn finite(value: Option<f64>) -> Option<f64> {
    value.filter(|number| number.is_finite())
}

fn classification_is_official(year: u16, indicator_id: u8) -> bool {
    !(matches!(year, 2020..=2022) || (year == 2023 && indicator_id == 5))
}

fn year_caveat(year: u16, indicator_id: u8) -> (Option<String>, bool) {
    if matches!(year, 2020 | 2021) {
        return (
            Some(
                "Information-only reporting year; do not interpret values as an official performance classification."
                    .to_owned(),
            ),
            true,
        );
    }
    if year == 2022 {
        return (
            Some(
                "Status-only reporting year; do not infer an official change or performance color."
                    .to_owned(),
            ),
            false,
        );
    }
    if year == 2023 && indicator_id == 5 {
        return (
            Some(
                "The College/Career Indicator had no official performance color in 2023."
                    .to_owned(),
            ),
            false,
        );
    }
    if year == 2025 && indicator_id == 8 {
        return (
            Some(
                "The 2025 Science indicator publishes status, change, and colors for informational use and is not used for accountability."
                    .to_owned(),
            ),
            true,
        );
    }
    (None, false)
}

fn quality_from_rows(rows: &[CanonicalRow]) -> DataQuality {
    let mut quality = DataQuality {
        rows_total: rows.len(),
        ..DataQuality::default()
    };
    for row in rows {
        if row.status.is_some() {
            quality.rows_reported += 1;
        }
        match row.missing_reason {
            Some(MissingReason::PrivateData) => quality.rows_private += 1,
            Some(MissingReason::SmallN) => quality.rows_small_n_suppressed += 1,
            Some(_) => quality.rows_missing += 1,
            None => {}
        }
        if row.small_n_warning {
            quality.rows_small_n_warning += 1;
        }
        if row.comparator_missing_reason.is_some() {
            quality.comparators_missing_or_suppressed += 1;
        }
    }
    quality
}

fn narrative_from_rows(rows: &[CanonicalRow]) -> Vec<NarrativeStatement> {
    let mut narrative = rows
        .iter()
        .take(NARRATIVE_ROW_LIMIT)
        .map(|row| NarrativeStatement {
            trace_id: row.trace_id.clone(),
            text: narrative_for_row(row),
        })
        .collect::<Vec<_>>();
    if rows.len() > NARRATIVE_ROW_LIMIT {
        narrative.push(NarrativeStatement {
            trace_id: "NARRATIVE_SCOPE".to_owned(),
            text: format!(
                "Narrative text covers {} of {} rows in CDS/year/indicator order; the data table retains every row.",
                NARRATIVE_ROW_LIMIT,
                rows.len()
            ),
        });
    }
    narrative
}

fn narrative_for_row(row: &CanonicalRow) -> String {
    let Some(status) = row.status else {
        let reason = row.missing_reason.map_or("MISSING", MissingReason::code);
        return format!(
            "{}: {} ({}) has no reportable {} value [{}].",
            row.trace_id, row.school_name, row.year, row.indicator_name, reason
        );
    };

    let mut text = format!(
        "{}: {} ({}) reported {} {} for {}",
        row.trace_id,
        row.school_name,
        row.year,
        format_number(status),
        row.status_unit,
        row.indicator_name
    );
    if let Some(change) = row.official_change {
        text.push_str(&format!(
            "; the official change field was {} {}",
            format_number(change),
            row.change_unit
        ));
    }
    if let (Some(raw), Some(favorable)) = (row.raw_comparator_gap, row.favorable_comparator_gap) {
        text.push_str(&format!(
            "; the raw comparator gap was {} {} and the direction-adjusted gap was {} {} ({})",
            format_number(raw),
            row.status_unit,
            format_number(favorable),
            row.status_unit,
            row.favorable_direction.label()
        ));
    }
    if row.small_n_warning {
        text.push_str("; the known denominator is 11–29, so the value should be read cautiously");
    }
    text.push('.');
    text
}

fn format_number(value: f64) -> String {
    let mut value = format!("{value:.3}");
    while value.contains('.') && value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    if value == "-0" { "0".to_owned() } else { value }
}

fn fetch_log_from_card(card: &SummaryCard) -> FetchLogEntry {
    FetchLogEntry {
        school_name: card.school.school_name.clone(),
        cds_code: card.school.cds_code.clone(),
        year: card.year,
        dashboard_year_id: card.dashboard_year_id,
        status: "SUCCESS".to_owned(),
        detail: String::new(),
        source_url: card.provenance.source_url.clone(),
        retrieved_at_utc: card.provenance.retrieved_at_utc.clone(),
        payload_sha256: card.provenance.payload_sha256.clone(),
        payload_bytes: card.provenance.payload_bytes,
        http_status: card.provenance.http_status,
        attempts: card.provenance.attempts,
        method_version: card.provenance.method_version.clone(),
    }
}

fn fetch_log_from_outcome(outcome: &FetchOutcome) -> FetchLogEntry {
    let (status, detail) = fetch_status_text(&outcome.status);
    FetchLogEntry {
        school_name: outcome.spec.school.school_name.clone(),
        cds_code: outcome.spec.school.cds_code.clone(),
        year: outcome.spec.year,
        dashboard_year_id: outcome.spec.dashboard_year_id,
        status: status.to_owned(),
        detail,
        source_url: outcome.provenance.source_url.clone(),
        retrieved_at_utc: outcome.provenance.retrieved_at_utc.clone(),
        payload_sha256: outcome.provenance.payload_sha256.clone(),
        payload_bytes: outcome.provenance.payload_bytes,
        http_status: outcome.provenance.http_status,
        attempts: outcome.provenance.attempts,
        method_version: outcome.provenance.method_version.clone(),
    }
}

fn fetch_status_text(status: &FetchStatus) -> (&'static str, String) {
    match status {
        FetchStatus::Success => ("SUCCESS", String::new()),
        FetchStatus::HttpError { status_code } => {
            ("HTTP_ERROR", format!("HTTP status {status_code}"))
        }
        FetchStatus::TransportError { message } => ("TRANSPORT_ERROR", message.clone()),
        FetchStatus::PayloadTooLarge {
            limit_bytes,
            declared_bytes,
        } => (
            "PAYLOAD_TOO_LARGE",
            declared_bytes.map_or_else(
                || format!("Limit {limit_bytes} bytes"),
                |size| format!("Declared {size} bytes; limit {limit_bytes} bytes"),
            ),
        ),
        FetchStatus::EmptyPayload => ("EMPTY_PAYLOAD", String::new()),
        FetchStatus::InvalidJson { message } => ("INVALID_JSON", message.clone()),
    }
}

fn methods_text() -> Vec<String> {
    vec![
        "One privacy-filtered canonical row set supplies the UI, CSV, workbook, HTML, and PDF outputs.".to_owned(),
        "A measure marked private or with a known denominator of 10 or fewer is suppressed in full; its numeric fields do not enter the canonical data.".to_owned(),
        "Published measures with a known denominator from 11 through 29 carry a caution flag.".to_owned(),
        "Raw comparator gap equals school status minus comparison status. Direction-adjusted gap reverses the sign only for indicators where lower values are favorable.".to_owned(),
        "Change is copied from the official API field and is not derived from adjacent years.".to_owned(),
        "Only direct descriptive values and arithmetic differences are reported. The output does not estimate uncertainty, order schools, combine indicators, or attribute mechanisms.".to_owned(),
        "Rows and narrative are ordered by CDS code, year, and indicator ID so selection is independent of observed values.".to_owned(),
    ]
}

fn sources_text() -> Vec<String> {
    vec![
        "California School Dashboard: https://www.caschooldashboard.org/".to_owned(),
        "California School Dashboard API request URLs are recorded per row and in the fetch log.".to_owned(),
        "California Department of Education public school directory: https://www.cde.ca.gov/ds/si/ds/pubschls.asp".to_owned(),
        format!("Reporting method version: {METHOD_VERSION}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Indicator, Provenance, SchoolRecord};

    fn card(measure: Measure, year: u16, indicator_id: u8) -> SummaryCard {
        SummaryCard {
            school: SchoolRecord {
                cds_code: "00123456789012".to_owned(),
                school_name: "Example School".to_owned(),
                county: None,
                district: None,
                city: None,
            },
            year,
            dashboard_year_id: 11,
            indicators: vec![Indicator {
                indicator_id,
                category: "test".to_owned(),
                primary_json: serde_json::Value::Null,
                secondary_json: serde_json::Value::Null,
                primary: Some(measure),
                secondary: Some(Measure {
                    status: Some(50.0),
                    count: Some(1_000),
                    ..Measure::default()
                }),
            }],
            provenance: Provenance {
                source_url: "https://example.test".to_owned(),
                retrieved_at_utc: "2026-01-01T00:00:00Z".to_owned(),
                payload_sha256: Some("abc".to_owned()),
                payload_bytes: 10,
                http_status: Some(200),
                attempts: 1,
                method_version: METHOD_VERSION.to_owned(),
            },
            raw_data: String::new(),
            raw_json_data: serde_json::Value::Null,
        }
    }

    #[test]
    fn privacy_and_small_counts_remove_all_primary_numbers() {
        for measure in [
            Measure {
                status: Some(987_654.321),
                change: Some(876_543.21),
                count: Some(300),
                performance: Some(5),
                is_private_data: Some(true),
                ..Measure::default()
            },
            Measure {
                status: Some(987_654.321),
                change: Some(876_543.21),
                count: Some(10),
                performance: Some(5),
                is_private_data: Some(false),
                ..Measure::default()
            },
        ] {
            let report = ReportModel::from_cards(&[card(measure, 2024, 2)]);
            let row = &report.rows[1];
            assert!(row.status.is_none());
            assert!(row.official_change.is_none());
            assert!(row.count.is_none());
            assert!(row.performance.is_none());
            assert!(row.raw_comparator_gap.is_none());
        }
    }

    #[test]
    fn gaps_have_raw_and_direction_adjusted_signs() {
        let report = ReportModel::from_cards(&[card(
            Measure {
                status: Some(40.0),
                count: Some(30),
                ..Measure::default()
            },
            2024,
            2,
        )]);
        let suspension = &report.rows[1];
        assert_eq!(suspension.raw_comparator_gap, Some(-10.0));
        assert_eq!(suspension.favorable_comparator_gap, Some(10.0));
    }

    #[test]
    fn all_indicator_definitions_are_present() {
        let definitions = indicator_definitions();
        assert_eq!(definitions.len(), 8);
        assert_eq!(
            definitions[0].favorable_direction,
            FavorableDirection::Lower
        );
        assert_eq!(definitions[7].status_unit, "science points (0–100)");
    }

    #[test]
    fn year_specific_fields_follow_published_dashboard_rules() {
        let measure = Measure {
            status: Some(55.0),
            change: Some(2.0),
            change_id: Some(3),
            performance: Some(4),
            red: Some(1),
            green: Some(2),
            count: Some(100),
            ..Measure::default()
        };
        let report_2022 = ReportModel::from_cards(&[card(measure.clone(), 2022, 8)]);
        let science_2022 = &report_2022.rows[7];
        assert_eq!(science_2022.status, Some(55.0));
        assert_eq!(science_2022.official_change, None);
        assert_eq!(science_2022.change_id, None);
        assert_eq!(science_2022.performance, None);
        assert_eq!(science_2022.red, None);

        let report_2025 = ReportModel::from_cards(&[card(measure, 2025, 8)]);
        let science_2025 = &report_2025.rows[7];
        assert_eq!(science_2025.official_change, Some(2.0));
        assert_eq!(science_2025.performance, Some(4));
        assert_eq!(science_2025.red, Some(1));
        assert!(science_2025.informational_only);
    }

    #[test]
    fn only_a_nonzero_performance_level_is_displayed() {
        // A performance level of 0 means "no color" and must not be displayed,
        // even though the status value is present and reportable.
        let zero = ReportModel::from_cards(&[card(
            Measure {
                status: Some(88.0),
                performance: Some(0),
                count: Some(500),
                ..Measure::default()
            },
            2024,
            4,
        )]);
        let graduation = &zero.rows[3];
        assert_eq!(graduation.status, Some(88.0));
        assert_eq!(graduation.performance, None);

        let nonzero = ReportModel::from_cards(&[card(
            Measure {
                status: Some(88.0),
                performance: Some(5),
                count: Some(500),
                ..Measure::default()
            },
            2024,
            4,
        )]);
        assert_eq!(nonzero.rows[3].performance, Some(5));
    }
}
