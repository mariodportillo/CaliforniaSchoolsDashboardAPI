//! Rendering of the canonical [`ReportModel`] into CSV, Excel, HTML, and PDF.
//!
//! Every output consumes the same privacy-filtered rows produced by
//! [`crate::statistics`].  No exporter re-interprets the Dashboard payload, so a
//! value suppressed for the browser is suppressed identically in every file.  A
//! missing value renders as an empty cell; a valid zero renders as `0`.

use std::collections::BTreeSet;

use rust_xlsxwriter::{
    Format, Formula, Table, TableColumn, TableFunction, TableStyle, Workbook, Worksheet,
};
use thiserror::Error;

use crate::model::DirectoryRecord;
use crate::statistics::{
    CanonicalRow, DataQuality, FavorableDirection, FetchLogEntry, IndicatorDefinition, ReportModel,
};
use crate::{APP_VERSION, METHOD_VERSION};

/// The four rendered artifacts for a single report.
///
/// The web layer serves these directly and the CLI writes them to disk; both
/// treat the bytes as opaque.
#[derive(Debug, Clone)]
pub struct ExportBundle {
    pub csv_bytes: Vec<u8>,
    pub xlsx_bytes: Vec<u8>,
    pub html_string: String,
    pub pdf_bytes: Vec<u8>,
}

/// A failure while rendering one of the export formats.
#[derive(Debug, Error)]
pub enum ExportError {
    #[error("CSV export failed: {0}")]
    Csv(String),
    #[error("Excel export failed: {0}")]
    Xlsx(#[from] rust_xlsxwriter::XlsxError),
}

impl ExportBundle {
    /// Renders all four formats from one canonical report.
    pub fn from_report(report: &ReportModel) -> Result<Self, ExportError> {
        Ok(Self {
            csv_bytes: csv_bytes(report)?,
            xlsx_bytes: xlsx_bytes(report)?,
            html_string: html_string(report),
            pdf_bytes: pdf_bytes(report),
        })
    }
}

/// A single presentation cell that preserves the missing-versus-zero distinction.
#[derive(Debug, Clone)]
enum Cell {
    /// No value was available; renders as an empty CSV field or a dash in HTML.
    Empty,
    Text(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Cell {
    fn opt_text(value: Option<&str>) -> Self {
        match value {
            Some(text) if !text.is_empty() => Self::Text(text.to_owned()),
            _ => Self::Empty,
        }
    }

    fn opt_float(value: Option<f64>) -> Self {
        value
            .filter(|number| number.is_finite())
            .map_or(Self::Empty, Self::Float)
    }

    fn opt_int<T: Into<i64>>(value: Option<T>) -> Self {
        value.map_or(Self::Empty, |number| Self::Int(number.into()))
    }

    /// Machine-readable rendering used for CSV.
    fn csv_value(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Text(text) => csv_safe_text(text),
            Self::Int(number) => number.to_string(),
            Self::Float(number) => format_number(*number),
            Self::Bool(flag) => if *flag { "true" } else { "false" }.to_owned(),
        }
    }

    /// Human-readable rendering used for HTML and PDF.
    fn display_value(&self) -> String {
        match self {
            Self::Empty => "—".to_owned(),
            Self::Bool(flag) => if *flag { "Yes" } else { "No" }.to_owned(),
            other => other.csv_value(),
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }
}

/// Prevents spreadsheet programs from interpreting untrusted CSV text as a formula.
fn csv_safe_text(text: &str) -> String {
    if text
        .chars()
        .next()
        .is_some_and(|first| matches!(first, '=' | '+' | '-' | '@' | '\t' | '\r'))
    {
        format!("'{text}")
    } else {
        text.to_owned()
    }
}

/// Stable, ordered column set shared by CSV, Excel, and HTML.
fn row_cells(row: &CanonicalRow) -> Vec<(&'static str, Cell)> {
    vec![
        ("trace_id", Cell::Text(row.trace_id.clone())),
        ("school_name", Cell::Text(row.school_name.clone())),
        // Kept as text so Excel preserves 14-digit CDS codes with leading zeroes.
        ("cds_code", Cell::Text(row.cds_code.clone())),
        ("year", Cell::Int(row.year.into())),
        ("dashboard_year_id", Cell::Int(row.dashboard_year_id.into())),
        ("indicator_id", Cell::Int(row.indicator_id.into())),
        ("category", Cell::Text(row.category.clone())),
        ("indicator_name", Cell::Text(row.indicator_name.clone())),
        ("status_unit", Cell::Text(row.status_unit.clone())),
        ("change_unit", Cell::Text(row.change_unit.clone())),
        (
            "favorable_direction",
            Cell::Text(favorable_direction_word(row.favorable_direction).to_owned()),
        ),
        ("status", Cell::opt_float(row.status)),
        ("official_change", Cell::opt_float(row.official_change)),
        ("change_id", Cell::opt_int(row.change_id)),
        ("status_id", Cell::opt_int(row.status_id)),
        ("performance", Cell::opt_int(row.performance)),
        ("total_groups", Cell::opt_int(row.total_groups)),
        ("red", Cell::opt_int(row.red)),
        ("orange", Cell::opt_int(row.orange)),
        ("yellow", Cell::opt_int(row.yellow)),
        ("green", Cell::opt_int(row.green)),
        ("blue", Cell::opt_int(row.blue)),
        ("count", Cell::opt_int(row.count.map(|value| value as i64))),
        (
            "student_group",
            Cell::opt_text(row.student_group.as_deref()),
        ),
        ("comparator_status", Cell::opt_float(row.comparator_status)),
        (
            "comparator_count",
            Cell::opt_int(row.comparator_count.map(|value| value as i64)),
        ),
        (
            "raw_comparator_gap",
            Cell::opt_float(row.raw_comparator_gap),
        ),
        (
            "favorable_comparator_gap",
            Cell::opt_float(row.favorable_comparator_gap),
        ),
        (
            "missing_reason",
            Cell::opt_text(row.missing_reason.map(|reason| reason.code())),
        ),
        (
            "comparator_missing_reason",
            Cell::opt_text(row.comparator_missing_reason.map(|reason| reason.code())),
        ),
        ("small_n_warning", Cell::Bool(row.small_n_warning)),
        (
            "comparator_small_n_warning",
            Cell::Bool(row.comparator_small_n_warning),
        ),
        ("year_caveat", Cell::opt_text(row.year_caveat.as_deref())),
        ("informational_only", Cell::Bool(row.informational_only)),
        ("source_url", Cell::Text(row.source_url.clone())),
        ("retrieved_at_utc", Cell::Text(row.retrieved_at_utc.clone())),
        (
            "payload_sha256",
            Cell::opt_text(row.payload_sha256.as_deref()),
        ),
        ("method_version", Cell::Text(row.method_version.clone())),
    ]
}

const fn favorable_direction_word(direction: FavorableDirection) -> &'static str {
    match direction {
        FavorableDirection::Higher => "higher",
        FavorableDirection::Lower => "lower",
    }
}

/// Formats a finite number with up to three decimals, trimming trailing zeroes.
/// Mirrors the narrative formatter so every surface agrees on a value's text.
fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return String::new();
    }
    let mut text = format!("{value:.3}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" { "0".to_owned() } else { text }
}

// --------------------------------------------------------------------------
// CSV
// --------------------------------------------------------------------------

fn csv_bytes(report: &ReportModel) -> Result<Vec<u8>, ExportError> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    let headers = column_headers();
    writer
        .write_record(&headers)
        .map_err(|error| ExportError::Csv(error.to_string()))?;
    for row in &report.rows {
        let record: Vec<String> = row_cells(row)
            .into_iter()
            .map(|(_, cell)| cell.csv_value())
            .collect();
        writer
            .write_record(&record)
            .map_err(|error| ExportError::Csv(error.to_string()))?;
    }
    writer
        .into_inner()
        .map_err(|error| ExportError::Csv(error.to_string()))
}

/// Renders the exact five-column CDE school/district directory CSV shape.
pub fn directory_csv_bytes(records: &[DirectoryRecord]) -> Result<Vec<u8>, ExportError> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer
        .write_record(["Record Type", "CDS Code", "District", "School", "Status"])
        .map_err(|error| ExportError::Csv(error.to_string()))?;
    for record in records {
        writer
            .write_record([
                csv_safe_text(&record.record_type),
                csv_safe_text(&record.cds_code),
                csv_safe_text(&record.district),
                csv_safe_text(&record.school),
                csv_safe_text(&record.status),
            ])
            .map_err(|error| ExportError::Csv(error.to_string()))?;
    }
    writer
        .into_inner()
        .map_err(|error| ExportError::Csv(error.to_string()))
}

fn column_headers() -> Vec<&'static str> {
    // Building from an empty template keeps the header list in lockstep with
    // `row_cells` without needing a representative row.
    let template = CanonicalRow {
        trace_id: String::new(),
        school_name: String::new(),
        cds_code: String::new(),
        year: 0,
        dashboard_year_id: 0,
        indicator_id: 0,
        category: String::new(),
        indicator_name: String::new(),
        status_unit: String::new(),
        change_unit: String::new(),
        favorable_direction: FavorableDirection::Higher,
        status: None,
        official_change: None,
        change_id: None,
        status_id: None,
        performance: None,
        total_groups: None,
        red: None,
        orange: None,
        yellow: None,
        green: None,
        blue: None,
        count: None,
        student_group: None,
        comparator_status: None,
        comparator_count: None,
        raw_comparator_gap: None,
        favorable_comparator_gap: None,
        missing_reason: None,
        comparator_missing_reason: None,
        small_n_warning: false,
        comparator_small_n_warning: false,
        year_caveat: None,
        informational_only: false,
        source_url: String::new(),
        retrieved_at_utc: String::new(),
        payload_sha256: None,
        method_version: String::new(),
    };
    row_cells(&template)
        .into_iter()
        .map(|(header, _)| header)
        .collect()
}

// --------------------------------------------------------------------------
// Excel
// --------------------------------------------------------------------------

fn xlsx_bytes(report: &ReportModel) -> Result<Vec<u8>, ExportError> {
    let mut workbook = Workbook::new();
    write_data_sheet(&mut workbook, report)?;
    write_quality_sheet(&mut workbook, &report.quality)?;
    write_definitions_sheet(&mut workbook, &report.definitions)?;
    write_fetch_log_sheet(&mut workbook, &report.fetch_log)?;
    write_text_sheet(&mut workbook, "Methods", "Method", &report.methods)?;
    write_text_sheet(&mut workbook, "Sources", "Source", &report.sources)?;
    Ok(workbook.save_to_buffer()?)
}

/// Renders a CDE-style `School and District Data` workbook with an actual
/// filterable Excel table and a formula-driven total row.
pub fn directory_xlsx_bytes(records: &[DirectoryRecord]) -> Result<Vec<u8>, ExportError> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("School and District Data")?;

    for (index, record) in records.iter().enumerate() {
        let row = index as u32 + 1;
        worksheet.write_string(row, 0, &record.record_type)?;
        worksheet.write_string(row, 1, &record.cds_code)?;
        worksheet.write_string(row, 2, &record.district)?;
        worksheet.write_string(row, 3, &record.school)?;
        worksheet.write_string(row, 4, &record.status)?;
    }

    let count_formula = if records.is_empty() {
        Formula::new("0")
    } else {
        Formula::new(format!("COUNTA(A2:A{})", records.len() + 1))
    };
    let columns = vec![
        TableColumn::new()
            .set_header("Record Type")
            .set_total_label("Total Records ="),
        TableColumn::new()
            .set_header("CDS Code")
            .set_total_function(TableFunction::Custom(count_formula)),
        TableColumn::new().set_header("District"),
        TableColumn::new().set_header("School"),
        TableColumn::new().set_header("Status"),
    ];
    let table = Table::new()
        .set_name("tblExport1")
        .set_style(TableStyle::Medium9)
        .set_columns(&columns)
        .set_total_row(true);
    worksheet.add_table(0, 0, records.len() as u32 + 1, 4, &table)?;
    worksheet.set_freeze_panes(1, 0)?;
    worksheet.set_column_width(0, 14)?;
    worksheet.set_column_width(1, 17)?;
    worksheet.set_column_width(2, 46)?;
    worksheet.set_column_width(3, 48)?;
    worksheet.set_column_width(4, 12)?;
    Ok(workbook.save_to_buffer()?)
}

fn write_cell(
    worksheet: &mut Worksheet,
    row: u32,
    col: u16,
    cell: &Cell,
) -> Result<(), ExportError> {
    match cell {
        Cell::Empty => {}
        Cell::Text(text) => {
            worksheet.write_string(row, col, text)?;
        }
        Cell::Int(number) => {
            worksheet.write_number(row, col, *number as f64)?;
        }
        Cell::Float(number) => {
            worksheet.write_number(row, col, *number)?;
        }
        Cell::Bool(flag) => {
            worksheet.write_boolean(row, col, *flag)?;
        }
    }
    Ok(())
}

fn write_headers(
    worksheet: &mut Worksheet,
    headers: &[&str],
    bold: &Format,
) -> Result<(), ExportError> {
    for (col, header) in headers.iter().enumerate() {
        worksheet.write_string_with_format(0, col as u16, *header, bold)?;
    }
    worksheet.set_row_height(0, 22)?;
    worksheet.set_freeze_panes(1, 0)?;
    Ok(())
}

fn header_format() -> Format {
    Format::new()
        .set_bold()
        .set_font_color("#FFFFFF")
        .set_background_color("#1F4E78")
}

fn write_data_sheet(workbook: &mut Workbook, report: &ReportModel) -> Result<(), ExportError> {
    let bold = header_format();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Data")?;
    let headers = column_headers();
    write_headers(worksheet, &headers, &bold)?;
    for (index, row) in report.rows.iter().enumerate() {
        let excel_row = index as u32 + 1;
        for (col, (_, cell)) in row_cells(row).into_iter().enumerate() {
            write_cell(worksheet, excel_row, col as u16, &cell)?;
        }
    }
    if !report.rows.is_empty() {
        let columns: Vec<_> = headers
            .iter()
            .map(|header| TableColumn::new().set_header(*header))
            .collect();
        let table = Table::new()
            .set_name("DashboardReportData")
            .set_style(TableStyle::Medium9)
            .set_columns(&columns);
        worksheet.add_table(
            0,
            0,
            report.rows.len() as u32,
            headers.len() as u16 - 1,
            &table,
        )?;
    }
    let decimal_format = Format::new().set_num_format("0.000");
    for column in [11, 12, 24, 26, 27] {
        worksheet.set_column_format(column, &decimal_format)?;
    }
    let widths = [
        27.0, 32.0, 16.0, 10.0, 18.0, 13.0, 30.0, 27.0, 24.0, 20.0, 20.0, 13.0, 17.0, 12.0, 12.0,
        14.0, 14.0, 10.0, 10.0, 10.0, 10.0, 10.0, 12.0, 18.0, 20.0, 18.0, 20.0, 24.0, 24.0, 28.0,
        18.0, 27.0, 58.0, 18.0, 66.0, 24.0, 66.0, 28.0,
    ];
    for (column, width) in widths.into_iter().enumerate() {
        worksheet.set_column_width(column as u16, width)?;
    }
    worksheet.set_freeze_panes(1, 3)?;
    Ok(())
}

fn write_quality_sheet(workbook: &mut Workbook, quality: &DataQuality) -> Result<(), ExportError> {
    let bold = header_format();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Data quality")?;
    write_headers(worksheet, &["Metric", "Value"], &bold)?;
    for (index, (label, value)) in quality_metrics(quality).into_iter().enumerate() {
        let row = index as u32 + 1;
        worksheet.write_string(row, 0, label)?;
        worksheet.write_number(row, 1, value as f64)?;
    }
    worksheet.set_column_width(0, 42)?;
    worksheet.set_column_width(1, 14)?;
    Ok(())
}

fn write_definitions_sheet(
    workbook: &mut Workbook,
    definitions: &[IndicatorDefinition],
) -> Result<(), ExportError> {
    let bold = header_format();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Indicator definitions")?;
    let headers = [
        "indicator_id",
        "category",
        "display_name",
        "status_unit",
        "change_unit",
        "favorable_direction",
        "description",
    ];
    write_headers(worksheet, &headers, &bold)?;
    for (index, definition) in definitions.iter().enumerate() {
        let row = index as u32 + 1;
        worksheet.write_number(row, 0, definition.indicator_id as f64)?;
        worksheet.write_string(row, 1, &definition.category)?;
        worksheet.write_string(row, 2, &definition.display_name)?;
        worksheet.write_string(row, 3, &definition.status_unit)?;
        worksheet.write_string(row, 4, &definition.change_unit)?;
        worksheet.write_string(
            row,
            5,
            favorable_direction_word(definition.favorable_direction),
        )?;
        worksheet.write_string(row, 6, &definition.description)?;
    }
    for (column, width) in [12.0, 30.0, 28.0, 24.0, 20.0, 20.0, 78.0]
        .into_iter()
        .enumerate()
    {
        worksheet.set_column_width(column as u16, width)?;
    }
    Ok(())
}

fn write_fetch_log_sheet(
    workbook: &mut Workbook,
    entries: &[FetchLogEntry],
) -> Result<(), ExportError> {
    let bold = header_format();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Fetch log")?;
    let headers = [
        "school_name",
        "cds_code",
        "year",
        "dashboard_year_id",
        "status",
        "detail",
        "http_status",
        "attempts",
        "payload_bytes",
        "payload_sha256",
        "source_url",
        "retrieved_at_utc",
        "method_version",
    ];
    write_headers(worksheet, &headers, &bold)?;
    for (index, entry) in entries.iter().enumerate() {
        let row = index as u32 + 1;
        worksheet.write_string(row, 0, &entry.school_name)?;
        worksheet.write_string(row, 1, &entry.cds_code)?;
        worksheet.write_number(row, 2, entry.year as f64)?;
        worksheet.write_number(row, 3, entry.dashboard_year_id as f64)?;
        worksheet.write_string(row, 4, &entry.status)?;
        worksheet.write_string(row, 5, &entry.detail)?;
        if let Some(http_status) = entry.http_status {
            worksheet.write_number(row, 6, http_status as f64)?;
        }
        worksheet.write_number(row, 7, entry.attempts as f64)?;
        worksheet.write_number(row, 8, entry.payload_bytes as f64)?;
        if let Some(hash) = &entry.payload_sha256 {
            worksheet.write_string(row, 9, hash)?;
        }
        worksheet.write_string(row, 10, &entry.source_url)?;
        worksheet.write_string(row, 11, &entry.retrieved_at_utc)?;
        worksheet.write_string(row, 12, &entry.method_version)?;
    }
    worksheet.set_column_width(0, 32)?;
    worksheet.set_column_width(1, 16)?;
    worksheet.set_column_width(4, 18)?;
    worksheet.set_column_width(5, 38)?;
    worksheet.set_column_width(9, 66)?;
    worksheet.set_column_width(10, 60)?;
    worksheet.set_column_width(11, 24)?;
    worksheet.set_column_width(12, 28)?;
    Ok(())
}

fn write_text_sheet(
    workbook: &mut Workbook,
    sheet_name: &str,
    header: &str,
    lines: &[String],
) -> Result<(), ExportError> {
    let bold = header_format();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name(sheet_name)?;
    write_headers(worksheet, &[header], &bold)?;
    for (index, line) in lines.iter().enumerate() {
        worksheet.write_string(index as u32 + 1, 0, line)?;
    }
    worksheet.set_column_width(0, 110)?;
    Ok(())
}

fn quality_metrics(quality: &DataQuality) -> Vec<(&'static str, u64)> {
    vec![
        ("Requests attempted", quality.fetches_total as u64),
        ("Requests succeeded", quality.fetches_succeeded as u64),
        ("Requests failed", quality.fetches_failed as u64),
        ("Summary cards parsed", quality.cards_total as u64),
        ("Rows generated", quality.rows_total as u64),
        ("Rows with a reportable value", quality.rows_reported as u64),
        ("Rows suppressed for privacy", quality.rows_private as u64),
        (
            "Rows suppressed for a small denominator (<=10)",
            quality.rows_small_n_suppressed as u64,
        ),
        (
            "Rows shown with a small-denominator caution (11-29)",
            quality.rows_small_n_warning as u64,
        ),
        (
            "Rows missing for other reasons",
            quality.rows_missing as u64,
        ),
        (
            "Comparators missing or suppressed",
            quality.comparators_missing_or_suppressed as u64,
        ),
        (
            "Duplicate indicators encountered",
            quality.duplicate_indicators as u64,
        ),
        (
            "Unexpected indicators retained",
            quality.unexpected_indicators as u64,
        ),
    ]
}

// --------------------------------------------------------------------------
// Report scope and shared narrative content
// --------------------------------------------------------------------------

struct ReportScope {
    school_count: usize,
    years: Vec<u16>,
}

fn report_scope(report: &ReportModel) -> ReportScope {
    let schools: BTreeSet<&str> = report
        .rows
        .iter()
        .map(|row| row.cds_code.as_str())
        .collect();
    let years: BTreeSet<u16> = report.rows.iter().map(|row| row.year).collect();
    ReportScope {
        school_count: schools.len(),
        years: years.into_iter().collect(),
    }
}

fn years_phrase(years: &[u16]) -> String {
    if years.is_empty() {
        return "none".to_owned();
    }
    years
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Fixed limitation statements drawn from the statistical reporting policy.
fn limitation_statements() -> Vec<&'static str> {
    vec![
        "These values are administrative, observational aggregates, not a random sample; no inferential statistics are produced.",
        "School populations, grade spans, and enrolled students can change between years, so a line across years is not a longitudinal student-cohort analysis.",
        "Missing or suppressed values may be nonrandom and are excluded rather than imputed.",
        "School-level aggregates can mask differences between student subgroups.",
        "No standard errors, confidence intervals, p-values, effect sizes, rankings, or composite scores are reported because the SummaryCards do not supply the raw counts, variances, or independent sample those methods require.",
        "The comparison (secondary) aggregate may include the selected school, which invalidates an independent two-sample test.",
    ]
}

// --------------------------------------------------------------------------
// HTML
// --------------------------------------------------------------------------

fn html_string(report: &ReportModel) -> String {
    let scope = report_scope(report);
    let mut html = String::new();
    html.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    html.push_str("<title>California School Dashboard — Descriptive Report</title>\n");
    html.push_str("<style>\n");
    html.push_str(HTML_STYLE);
    html.push_str("\n</style>\n</head>\n<body>\n");

    html.push_str("<header class=\"report-header\">\n");
    html.push_str("<h1>California School Dashboard — Descriptive Report</h1>\n");
    html.push_str(&format!(
        "<p class=\"scope\">{} school{} · reporting year{}: {} · {} data row{} · method {}</p>\n",
        scope.school_count,
        plural(scope.school_count),
        plural(scope.years.len()),
        escape_html(&years_phrase(&scope.years)),
        report.rows.len(),
        plural(report.rows.len()),
        escape_html(&report.method_version),
    ));
    html.push_str(&format!(
        "<p class=\"scope\">Generated by california-dashboard v{}. Values are descriptive administrative aggregates from the California School Dashboard.</p>\n",
        escape_html(APP_VERSION)
    ));
    html.push_str(&format!(
        "<a class=\"source-link\" href=\"https://www.caschooldashboard.org/\" target=\"_blank\" rel=\"noopener noreferrer\" aria-label=\"Official data source: California School Dashboard website (opens in a new tab)\"><span class=\"logo-box\">{DASHBOARD_LOGO_SVG}</span><span class=\"source-link-text\">Official public source: caschooldashboard.org &#8599;</span></a>\n"
    ));
    html.push_str("</header>\n");

    // Data completeness.
    html.push_str("<section>\n<h2>Data completeness</h2>\n<table class=\"summary\">\n");
    html.push_str("<thead><tr><th scope=\"col\">Metric</th><th scope=\"col\">Value</th></tr></thead>\n<tbody>\n");
    for (label, value) in quality_metrics(&report.quality) {
        html.push_str(&format!(
            "<tr><th scope=\"row\">{}</th><td class=\"num\">{}</td></tr>\n",
            escape_html(label),
            value
        ));
    }
    html.push_str("</tbody>\n</table>\n</section>\n");

    // Methods.
    html.push_str("<section>\n<h2>How these numbers were prepared</h2>\n<ul>\n");
    for method in &report.methods {
        html.push_str(&format!("<li>{}</li>\n", escape_html(method)));
    }
    html.push_str("</ul>\n</section>\n");

    // Limitations.
    html.push_str("<section>\n<h2>Limitations</h2>\n<ul>\n");
    for statement in limitation_statements() {
        html.push_str(&format!("<li>{}</li>\n", escape_html(statement)));
    }
    html.push_str("</ul>\n</section>\n");

    // Data table.
    html.push_str("<section>\n<h2>Reported measures</h2>\n");
    if report.rows.is_empty() {
        html.push_str("<p class=\"empty\">No rows were produced for this selection.</p>\n");
    } else {
        html.push_str("<div class=\"table-scroll\">\n<table class=\"data\">\n<thead><tr>");
        for header in column_headers() {
            html.push_str(&format!("<th scope=\"col\">{}</th>", escape_html(header)));
        }
        html.push_str("</tr></thead>\n<tbody>\n");
        for row in &report.rows {
            html.push_str("<tr>");
            for (_, cell) in row_cells(row) {
                let class = if cell.is_empty() {
                    " class=\"missing\""
                } else {
                    ""
                };
                html.push_str(&format!(
                    "<td{}>{}</td>",
                    class,
                    escape_html(&cell.display_value())
                ));
            }
            html.push_str("</tr>\n");
        }
        html.push_str("</tbody>\n</table>\n</div>\n");
    }
    html.push_str("</section>\n");

    // Narrative findings.
    if !report.narrative.is_empty() {
        html.push_str("<section>\n<h2>Row-by-row description</h2>\n<ol class=\"narrative\">\n");
        for statement in &report.narrative {
            html.push_str(&format!("<li>{}</li>\n", escape_html(&statement.text)));
        }
        html.push_str("</ol>\n</section>\n");
    }

    // Fetch log.
    html.push_str("<section>\n<h2>Retrieval log</h2>\n");
    if report.fetch_log.is_empty() {
        html.push_str("<p class=\"empty\">No requests were recorded.</p>\n");
    } else {
        html.push_str("<div class=\"table-scroll\">\n<table class=\"data\">\n<thead><tr>");
        for header in [
            "School",
            "CDS",
            "Year",
            "Status",
            "HTTP",
            "Attempts",
            "Detail",
            "Source URL",
        ] {
            html.push_str(&format!("<th scope=\"col\">{}</th>", escape_html(header)));
        }
        html.push_str("</tr></thead>\n<tbody>\n");
        for entry in &report.fetch_log {
            html.push_str("<tr>");
            html.push_str(&format!("<td>{}</td>", escape_html(&entry.school_name)));
            html.push_str(&format!("<td>{}</td>", escape_html(&entry.cds_code)));
            html.push_str(&format!("<td class=\"num\">{}</td>", entry.year));
            html.push_str(&format!("<td>{}</td>", escape_html(&entry.status)));
            html.push_str(&format!(
                "<td class=\"num\">{}</td>",
                entry
                    .http_status
                    .map_or_else(|| "—".to_owned(), |code| code.to_string())
            ));
            html.push_str(&format!("<td class=\"num\">{}</td>", entry.attempts));
            html.push_str(&format!("<td>{}</td>", escape_html(&entry.detail)));
            html.push_str(&format!("<td>{}</td>", escape_html(&entry.source_url)));
            html.push_str("</tr>\n");
        }
        html.push_str("</tbody>\n</table>\n</div>\n");
    }
    html.push_str("</section>\n");

    // Sources.
    html.push_str("<section>\n<h2>Sources</h2>\n<ul>\n");
    for source in &report.sources {
        html.push_str(&format!("<li>{}</li>\n", escape_html(source)));
    }
    html.push_str("</ul>\n</section>\n");

    // Research disclaimer.
    html.push_str("<footer class=\"report-footer\">\n<p class=\"disclaimer\"><strong>Research use only.</strong> ");
    html.push_str(&escape_html(RESEARCH_DISCLAIMER));
    html.push_str("</p>\n<p class=\"disclaimer-meta\">Independent, research-oriented tool — not affiliated with or endorsed by the State of California or the California Department of Education. Source: <a href=\"https://www.caschooldashboard.org/\">https://www.caschooldashboard.org/</a></p>\n</footer>\n");

    html.push_str("</body>\n</html>\n");
    html
}

const fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Shown at the bottom of every rendered report output (after a bold lead-in).
const RESEARCH_DISCLAIMER: &str = "This report is entirely research-based. Every value is derived from the public California School Dashboard and should be independently verified against the official source before being used for any decision.";

/// Inline recreation of the California School Dashboard gauge-and-pencil logo.
/// Kept self-contained (no external image) so reports render offline.
const DASHBOARD_LOGO_SVG: &str = r##"<svg viewBox="0 0 360 74" role="img" aria-label="California School Dashboard" xmlns="http://www.w3.org/2000/svg"><title>California School Dashboard</title><rect x="16" y="61" width="92" height="5" rx="2.5" fill="#2E3B4E"/><path d="M 22.00 60.00 A 40 40 0 0 1 28.38 38.33" fill="none" stroke="#E5342A" stroke-width="15"/><path d="M 29.97 36.04 A 40 40 0 0 1 48.06 22.51" fill="none" stroke="#F5871F" stroke-width="15"/><path d="M 50.71 21.63 A 40 40 0 0 1 73.29 21.63" fill="none" stroke="#FDC010" stroke-width="15"/><path d="M 75.94 22.51 A 40 40 0 0 1 94.03 36.04" fill="none" stroke="#4FA32E" stroke-width="15"/><path d="M 95.62 38.33 A 40 40 0 0 1 102.00 60.00" fill="none" stroke="#1C75BC" stroke-width="15"/><g transform="rotate(-20 62 60)"><polygon points="62,12 57.5,24 66.5,24" fill="#E9CF97"/><polygon points="62,12 60,17.5 64,17.5" fill="#33373D"/><rect x="57.5" y="24" width="9" height="22" fill="#F6B21B"/><rect x="57.5" y="46" width="9" height="4" fill="#C9CDD3"/><rect x="57.5" y="50" width="9" height="8.5" rx="1.5" fill="#EE8E8E"/></g><circle cx="62" cy="60" r="6" fill="#2E3B4E"/><circle cx="62" cy="60" r="2.4" fill="#ffffff"/><text x="128" y="31" font-family="Arial, Helvetica, sans-serif" font-size="18" font-weight="600" fill="#2E3B4E">California School</text><text x="127" y="60" font-family="Arial, Helvetica, sans-serif" font-size="30" font-weight="800" letter-spacing="0.5" fill="#2E3B4E">DASHBOARD</text></svg>"##;

const HTML_STYLE: &str = r#":root { color-scheme: light dark; }
* { box-sizing: border-box; }
body { font-family: -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif; margin: 0; padding: 2rem clamp(1rem, 4vw, 3rem); line-height: 1.5; color: #1a1a1a; background: #fff; }
.report-header { border-bottom: 3px solid #1a3a63; padding-bottom: 1rem; margin-bottom: 1.5rem; }
h1 { font-size: 1.6rem; margin: 0 0 0.5rem; color: #1a3a63; }
h2 { font-size: 1.15rem; margin: 2rem 0 0.75rem; color: #1a3a63; }
.scope { margin: 0.25rem 0; color: #555; font-size: 0.95rem; }
.source-link { display: inline-flex; align-items: center; gap: 0.75rem; margin-top: 0.75rem; text-decoration: none; color: #1a3a63; }
.source-link .logo-box { display: inline-block; background: #fff; border: 1px solid #d7deea; border-radius: 8px; padding: 6px 10px; line-height: 0; }
.source-link .logo-box svg { display: block; width: 180px; height: auto; }
.source-link-text { font-size: 0.85rem; font-weight: 600; text-decoration: underline; }
.source-link:hover .source-link-text { text-decoration: none; }
.report-footer { margin-top: 2.5rem; padding-top: 1rem; border-top: 2px solid #1a3a63; }
.disclaimer { font-size: 0.9rem; color: #333; margin: 0 0 0.5rem; }
.disclaimer strong { color: #1a3a63; }
.disclaimer-meta { font-size: 0.8rem; color: #666; margin: 0; }
.disclaimer-meta a { color: inherit; }
section { max-width: 100%; }
ul, ol { margin: 0.5rem 0; padding-left: 1.4rem; }
li { margin: 0.3rem 0; }
.narrative li { color: #333; font-size: 0.92rem; }
.table-scroll { overflow-x: auto; border: 1px solid #ddd; border-radius: 6px; }
table { border-collapse: collapse; width: 100%; font-size: 0.85rem; }
table.summary { max-width: 32rem; }
th, td { padding: 0.4rem 0.6rem; border-bottom: 1px solid #eee; text-align: left; white-space: nowrap; }
thead th { position: sticky; top: 0; background: #1a3a63; color: #fff; font-weight: 600; }
table.summary th[scope="row"] { font-weight: 500; }
td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
td.missing { color: #999; text-align: center; }
tbody tr:nth-child(even) { background: rgba(26, 58, 99, 0.04); }
.empty { color: #777; font-style: italic; }
@media (prefers-color-scheme: dark) {
  body { color: #e8e8e8; background: #121212; }
  h1, h2, .report-header { color: #8fb8e6; border-color: #2a4a73; }
  .scope { color: #aaa; }
  .table-scroll { border-color: #333; }
  th, td { border-color: #2a2a2a; }
  thead th { background: #1a3a63; }
  tbody tr:nth-child(even) { background: rgba(143, 184, 230, 0.06); }
}
"#;

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

// --------------------------------------------------------------------------
// PDF
// --------------------------------------------------------------------------

fn pdf_bytes(report: &ReportModel) -> Vec<u8> {
    let scope = report_scope(report);
    let mut document = PdfDocument::new();

    document.heading("California School Dashboard");
    document.heading_secondary("Descriptive Report");
    document.spacer();
    document.body(&format!(
        "Coverage: {} school{}, reporting year{} {}, {} data row{}.",
        scope.school_count,
        plural(scope.school_count),
        plural(scope.years.len()),
        years_phrase(&scope.years),
        report.rows.len(),
        plural(report.rows.len()),
    ));
    document.body(&format!(
        "Prepared by california-dashboard v{APP_VERSION} using reporting method {METHOD_VERSION}. Values are descriptive administrative aggregates and are not a random sample."
    ));
    document.spacer();

    document.subheading("Data completeness");
    for (label, value) in quality_metrics(&report.quality) {
        document.bullet(&format!("{label}: {value}"));
    }
    document.spacer();

    document.subheading("How these numbers were prepared");
    for method in &report.methods {
        document.bullet(method);
    }
    document.spacer();

    document.subheading("Limitations");
    for statement in limitation_statements() {
        document.bullet(statement);
    }
    document.spacer();

    document.subheading("Row-by-row description");
    if report.narrative.is_empty() {
        document.body("No reportable rows were produced for this selection.");
    } else {
        for statement in &report.narrative {
            document.bullet(&statement.text);
        }
    }
    document.spacer();

    document.subheading("Sources");
    for source in &report.sources {
        document.bullet(source);
    }
    document.spacer();

    document.subheading("Disclaimer");
    document.body(&format!("Research use only. {RESEARCH_DISCLAIMER}"));
    document.body(
        "Independent, research-oriented tool — not affiliated with or endorsed by the State of California or the California Department of Education. Source: https://www.caschooldashboard.org/",
    );

    document.render()
}

/// The two style/size choices used in the PDF report.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PdfFont {
    Regular,
    Bold,
}

struct PdfLine {
    text: String,
    size: f64,
    font: PdfFont,
    indent: f64,
}

/// A minimal, dependency-free PDF writer for left-aligned text reports.
///
/// It emits the standard Helvetica fonts (no embedding needed), wraps text to
/// the page width, and paginates automatically.  It is intentionally limited to
/// the text a descriptive report requires.
struct PdfDocument {
    lines: Vec<PdfLine>,
}

const PDF_PAGE_WIDTH: f64 = 612.0;
const PDF_PAGE_HEIGHT: f64 = 792.0;
const PDF_MARGIN: f64 = 54.0;
const PDF_CONTENT_WIDTH: f64 = PDF_PAGE_WIDTH - 2.0 * PDF_MARGIN;

impl PdfDocument {
    fn new() -> Self {
        Self { lines: Vec::new() }
    }

    fn heading(&mut self, text: &str) {
        self.push_wrapped(text, 18.0, PdfFont::Bold, 0.0);
    }

    fn heading_secondary(&mut self, text: &str) {
        self.push_wrapped(text, 13.0, PdfFont::Bold, 0.0);
    }

    fn subheading(&mut self, text: &str) {
        self.push_wrapped(text, 12.0, PdfFont::Bold, 0.0);
    }

    fn body(&mut self, text: &str) {
        self.push_wrapped(text, 10.0, PdfFont::Regular, 0.0);
    }

    fn bullet(&mut self, text: &str) {
        // The bullet glyph shares the first wrapped line; continuation lines are
        // indented to align under the text.
        let wrapped = wrap_text(text, 10.0, PdfFont::Regular, PDF_CONTENT_WIDTH - 12.0);
        for (index, segment) in wrapped.into_iter().enumerate() {
            let (rendered, indent) = if index == 0 {
                (format!("\u{2022}  {segment}"), 0.0)
            } else {
                (segment, 12.0)
            };
            self.lines.push(PdfLine {
                text: rendered,
                size: 10.0,
                font: PdfFont::Regular,
                indent,
            });
        }
    }

    fn spacer(&mut self) {
        self.lines.push(PdfLine {
            text: String::new(),
            size: 6.0,
            font: PdfFont::Regular,
            indent: 0.0,
        });
    }

    fn push_wrapped(&mut self, text: &str, size: f64, font: PdfFont, indent: f64) {
        for segment in wrap_text(text, size, font, PDF_CONTENT_WIDTH - indent) {
            self.lines.push(PdfLine {
                text: segment,
                size,
                font,
                indent,
            });
        }
    }

    /// Lays lines onto pages and serializes a complete PDF file.
    fn render(&self) -> Vec<u8> {
        let mut pages: Vec<Vec<u8>> = Vec::new();
        let mut current = Vec::new();
        let mut cursor = PDF_PAGE_HEIGHT - PDF_MARGIN;
        let bottom = PDF_MARGIN;

        for line in &self.lines {
            let line_height = line.size * 1.32;
            if cursor - line_height < bottom && !current.is_empty() {
                pages.push(std::mem::take(&mut current));
                cursor = PDF_PAGE_HEIGHT - PDF_MARGIN;
            }
            cursor -= line_height;
            if !line.text.is_empty() {
                append_text_command(
                    &mut current,
                    PDF_MARGIN + line.indent,
                    cursor,
                    line.size,
                    line.font,
                    &line.text,
                );
            }
        }
        pages.push(current);
        serialize_pdf(&pages)
    }
}

/// Wraps `text` to `max_width` points using an average Helvetica advance width.
fn wrap_text(text: &str, size: f64, font: PdfFont, max_width: f64) -> Vec<String> {
    let advance = average_advance(font) * size;
    let max_chars = ((max_width / advance).floor() as usize).max(8);
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        if word.chars().count() > max_chars {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            for chunk in hard_split(word, max_chars) {
                lines.push(chunk);
            }
            continue;
        }
        if current.is_empty() {
            current = word.to_owned();
        } else if current.chars().count() + 1 + word.chars().count() <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current = word.to_owned();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Conservative average glyph advance as a fraction of font size for Helvetica.
const fn average_advance(font: PdfFont) -> f64 {
    match font {
        PdfFont::Regular => 0.52,
        PdfFont::Bold => 0.56,
    }
}

fn hard_split(word: &str, max_chars: usize) -> Vec<String> {
    let characters: Vec<char> = word.chars().collect();
    characters
        .chunks(max_chars)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

fn append_text_command(
    content: &mut Vec<u8>,
    x: f64,
    y: f64,
    size: f64,
    font: PdfFont,
    text: &str,
) {
    let font_name = match font {
        PdfFont::Regular => "F1",
        PdfFont::Bold => "F2",
    };
    content.extend_from_slice(
        format!("BT\n/{font_name} {size:.2} Tf\n1 0 0 1 {x:.2} {y:.2} Tm\n(").as_bytes(),
    );
    content.extend_from_slice(&escape_pdf_text(text));
    content.extend_from_slice(b") Tj\nET\n");
}

/// Escapes a string for a PDF literal and maps to WinAnsi-safe bytes.
fn escape_pdf_text(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => bytes.extend_from_slice(b"\\\\"),
            '(' => bytes.extend_from_slice(b"\\("),
            ')' => bytes.extend_from_slice(b"\\)"),
            other => bytes.push(winansi_byte(other)),
        }
    }
    bytes
}

/// Maps a character to its WinAnsiEncoding byte, translating the common
/// typographic punctuation that lives in the `0x80`–`0x9F` block and passing
/// through the shared Latin-1 range.  Anything unrepresentable becomes `?`.
const fn winansi_byte(character: char) -> u8 {
    match character {
        '\u{2022}' => 0x95, // bullet
        '\u{2013}' => 0x96, // en dash
        '\u{2014}' => 0x97, // em dash
        '\u{2018}' => 0x91, // left single quote
        '\u{2019}' => 0x92, // right single quote / apostrophe
        '\u{201C}' => 0x93, // left double quote
        '\u{201D}' => 0x94, // right double quote
        '\u{2026}' => 0x85, // ellipsis
        '\u{2020}' => 0x86, // dagger
        '\u{2021}' => 0x87, // double dagger
        '\u{2030}' => 0x89, // per mille
        '\u{2122}' => 0x99, // trademark
        '\u{20AC}' => 0x80, // euro
        other => {
            let code = other as u32;
            // Printable ASCII and the Latin-1 range WinAnsi shares byte-for-byte.
            if (0x20 <= code && code <= 0x7E) || (0xA0 <= code && code <= 0xFF) {
                code as u8
            } else {
                b'?'
            }
        }
    }
}

fn serialize_pdf(pages: &[Vec<u8>]) -> Vec<u8> {
    let page_count = pages.len().max(1);
    // Object layout: 1 catalog, 2 pages, 3 font regular, 4 font bold, then a
    // page object and a content object for each page.
    let object_count = 4 + page_count * 2;
    let mut output = Vec::new();
    let mut offsets = vec![0usize; object_count + 1];

    output.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

    let kids: String = (0..page_count)
        .map(|index| format!("{} 0 R", 5 + index * 2))
        .collect::<Vec<_>>()
        .join(" ");

    let write_object = |output: &mut Vec<u8>, offsets: &mut [usize], id: usize, body: &[u8]| {
        offsets[id] = output.len();
        output.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        output.extend_from_slice(body);
        output.extend_from_slice(b"\nendobj\n");
    };

    write_object(
        &mut output,
        &mut offsets,
        1,
        b"<< /Type /Catalog /Pages 2 0 R >>",
    );
    write_object(
        &mut output,
        &mut offsets,
        2,
        format!("<< /Type /Pages /Kids [{kids}] /Count {page_count} >>").as_bytes(),
    );
    write_object(
        &mut output,
        &mut offsets,
        3,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );
    write_object(
        &mut output,
        &mut offsets,
        4,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>",
    );

    let empty = Vec::new();
    for index in 0..page_count {
        let page_id = 5 + index * 2;
        let content_id = page_id + 1;
        let page_body = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PDF_PAGE_WIDTH:.0} {PDF_PAGE_HEIGHT:.0}] \
             /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {content_id} 0 R >>"
        );
        write_object(&mut output, &mut offsets, page_id, page_body.as_bytes());

        let content = pages.get(index).unwrap_or(&empty);
        let mut content_object = Vec::new();
        content_object
            .extend_from_slice(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
        content_object.extend_from_slice(content);
        content_object.extend_from_slice(b"\nendstream");
        write_object(&mut output, &mut offsets, content_id, &content_object);
    }

    let xref_offset = output.len();
    output.extend_from_slice(format!("xref\n0 {}\n", object_count + 1).as_bytes());
    output.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            object_count + 1
        )
        .as_bytes(),
    );

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Indicator, Measure, Provenance, SchoolRecord, SummaryCard};
    use calamine::Reader;
    use std::io::Cursor;

    fn sample_report() -> ReportModel {
        let card = SummaryCard {
            school: SchoolRecord {
                cds_code: "01234567890123".to_owned(),
                school_name: "Ada <Lovelace> & Turing School".to_owned(),
                county: Some("Alameda".to_owned()),
                district: Some("Bay Unified".to_owned()),
                city: Some("Oakland".to_owned()),
            },
            year: 2024,
            dashboard_year_id: 10,
            indicators: vec![
                Indicator {
                    indicator_id: 1,
                    category: "CHRONIC_ABSENTEEISM".to_owned(),
                    primary_json: serde_json::Value::Null,
                    secondary_json: serde_json::Value::Null,
                    // A valid zero must survive as zero, not be dropped.
                    primary: Some(Measure {
                        status: Some(0.0),
                        change: Some(-1.5),
                        count: Some(500),
                        ..Measure::default()
                    }),
                    secondary: Some(Measure {
                        status: Some(7.5),
                        count: Some(200_000),
                        ..Measure::default()
                    }),
                },
                Indicator {
                    indicator_id: 2,
                    category: "SUSPENSION_RATE".to_owned(),
                    primary_json: serde_json::Value::Null,
                    secondary_json: serde_json::Value::Null,
                    // Private data must be fully suppressed.
                    primary: Some(Measure {
                        status: Some(3.0),
                        count: Some(50),
                        is_private_data: Some(true),
                        ..Measure::default()
                    }),
                    secondary: None,
                },
            ],
            provenance: Provenance {
                source_url:
                    "https://api.caschooldashboard.org/Reports/01234567890123/10/SummaryCards"
                        .to_owned(),
                retrieved_at_utc: "2026-07-09T00:00:00Z".to_owned(),
                payload_sha256: Some("deadbeef".to_owned()),
                payload_bytes: 128,
                http_status: Some(200),
                attempts: 1,
                method_version: METHOD_VERSION.to_owned(),
            },
            raw_data: String::new(),
            raw_json_data: serde_json::Value::Null,
        };
        ReportModel::from_cards(&[card])
    }

    #[test]
    fn bundle_produces_all_four_non_empty_formats() {
        let bundle = ExportBundle::from_report(&sample_report()).unwrap();
        assert!(!bundle.csv_bytes.is_empty());
        assert!(!bundle.xlsx_bytes.is_empty());
        assert!(!bundle.html_string.is_empty());
        assert!(!bundle.pdf_bytes.is_empty());
        assert!(bundle.xlsx_bytes.starts_with(b"PK"));
        assert!(bundle.pdf_bytes.starts_with(b"%PDF-1.7"));
        assert!(bundle.pdf_bytes.ends_with(b"%%EOF\n"));
        // The research disclaimer rides along on every rendered output.
        assert!(
            bundle
                .pdf_bytes
                .windows(b"Research use only.".len())
                .any(|window| window == b"Research use only.")
        );
        assert!(bundle.html_string.contains("Research use only."));
    }

    #[test]
    fn csv_preserves_zero_and_leaves_suppressed_values_blank() {
        let report = sample_report();
        let csv = String::from_utf8(csv_bytes(&report).unwrap()).unwrap();
        let mut reader = csv::Reader::from_reader(csv.as_bytes());
        let headers: Vec<String> = reader
            .headers()
            .unwrap()
            .iter()
            .map(str::to_owned)
            .collect();
        let status_col = headers.iter().position(|h| h == "status").unwrap();
        let count_col = headers.iter().position(|h| h == "count").unwrap();
        let indicator_col = headers.iter().position(|h| h == "indicator_id").unwrap();

        let mut absenteeism_zero = false;
        let mut suspension_blank = false;
        for record in reader.records() {
            let record = record.unwrap();
            match record.get(indicator_col) {
                Some("1") => {
                    assert_eq!(record.get(status_col), Some("0"));
                    assert_eq!(record.get(count_col), Some("500"));
                    absenteeism_zero = true;
                }
                Some("2") => {
                    // Private data: every numeric field is blank.
                    assert_eq!(record.get(status_col), Some(""));
                    assert_eq!(record.get(count_col), Some(""));
                    suspension_blank = true;
                }
                _ => {}
            }
        }
        assert!(absenteeism_zero && suspension_blank);
    }

    #[test]
    fn csv_neutralizes_formula_prefixes() {
        for dangerous in [
            "=2+2",
            "+SUM(A:A)",
            "-1+1",
            "@cmd",
            "\tformula",
            "\rformula",
        ] {
            assert_eq!(csv_safe_text(dangerous), format!("'{dangerous}"));
        }
        assert_eq!(csv_safe_text("ordinary"), "ordinary");
    }

    #[test]
    fn directory_exports_match_the_reference_schema() {
        let records = vec![DirectoryRecord {
            record_type: "District".to_owned(),
            cds_code: "01100170000000".to_owned(),
            district: "Alameda County Office of Education".to_owned(),
            school: "No Data".to_owned(),
            status: "Active".to_owned(),
        }];
        let csv = String::from_utf8(directory_csv_bytes(&records).unwrap()).unwrap();
        assert!(csv.starts_with("Record Type,CDS Code,District,School,Status\n"));
        assert!(csv.contains("District,01100170000000"));
        let xlsx = directory_xlsx_bytes(&records).unwrap();
        assert!(xlsx.starts_with(b"PK"));
        let mut workbook = calamine::open_workbook_auto_from_rs(Cursor::new(xlsx)).unwrap();
        let range = workbook
            .worksheet_range("School and District Data")
            .unwrap();
        assert_eq!(range.get_value((0, 0)).unwrap().to_string(), "Record Type");
        assert_eq!(
            range.get_value((1, 1)).unwrap().to_string(),
            "01100170000000"
        );
    }

    #[test]
    fn pdf_text_maps_typographic_glyphs_to_winansi() {
        // Bullet and en dash must become their WinAnsi bytes, not '?'.
        assert_eq!(escape_pdf_text("\u{2022}"), vec![0x95]);
        assert_eq!(
            escape_pdf_text("11\u{2013}29"),
            vec![b'1', b'1', 0x96, b'2', b'9']
        );
        // Parentheses and backslashes are escaped for the PDF literal.
        assert_eq!(escape_pdf_text("a(b)\\c"), b"a\\(b\\)\\\\c".to_vec());
        // An unrepresentable glyph falls back to '?'.
        assert_eq!(escape_pdf_text("\u{1F600}"), vec![b'?']);
    }

    #[test]
    fn html_escapes_dynamic_content() {
        let report = sample_report();
        let html = html_string(&report);
        assert!(html.contains("Ada &lt;Lovelace&gt; &amp; Turing School"));
        assert!(!html.contains("Ada <Lovelace>"));
        assert!(html.contains(&report.method_version));
    }

    #[test]
    fn report_html_has_working_source_link_logo_and_disclaimer() {
        let html = html_string(&sample_report());
        // A real, accessible hyperlink to the official source, opening safely.
        assert!(html.contains("href=\"https://www.caschooldashboard.org/\""));
        assert!(html.contains("rel=\"noopener noreferrer\""));
        // The source logo is embedded inline (renders offline, CSP-safe).
        assert!(html.contains("<title>California School Dashboard</title>"));
        // A research disclaimer is present at the bottom.
        assert!(html.contains("Research use only."));
        assert!(html.contains("should be independently verified"));
    }

    #[test]
    fn xlsx_round_trips_with_suppression_intact() {
        use calamine::{Data, Reader, Xlsx};

        let report = sample_report();
        let bytes = xlsx_bytes(&report).unwrap();
        let mut workbook: Xlsx<_> = calamine::open_workbook_from_rs(Cursor::new(bytes)).unwrap();
        let range = workbook.worksheet_range("Data").unwrap();

        let headers: Vec<String> = range
            .rows()
            .next()
            .unwrap()
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        let status_col = headers.iter().position(|h| h == "status").unwrap();
        let cds_col = headers.iter().position(|h| h == "cds_code").unwrap();
        let indicator_col = headers.iter().position(|h| h == "indicator_id").unwrap();

        for row in range.rows().skip(1) {
            let indicator = &row[indicator_col];
            // Leading zeros are preserved because CDS is stored as text.
            assert_eq!(row[cds_col], Data::String("01234567890123".to_owned()));
            if matches!(indicator, Data::Float(value) if *value == 1.0) {
                assert_eq!(row[status_col], Data::Float(0.0));
            }
            if matches!(indicator, Data::Float(value) if *value == 2.0) {
                assert!(matches!(row[status_col], Data::Empty));
            }
        }
    }
}
