//! California School Dashboard client, exports, and local web application.

pub mod client;
pub mod database;
pub mod export;
pub mod model;
pub mod resolver;
pub mod statistics;
pub mod web;
pub mod years;

pub use client::{ClientConfig, DashboardClient};
pub use database::{
    DatabaseValidation, FieldDifference, ImportSummary, RowDifference, import_school_csv,
    validate_school_database,
};
pub use export::{ExportBundle, ExportError, directory_csv_bytes, directory_xlsx_bytes};
pub use model::{
    DirectoryRecord, FetchOutcome, FetchSpec, FetchStatus, Indicator, Measure, Provenance,
    SchoolRecord, SummaryCard,
};
pub use resolver::{MatchKind, MatchResult, ResolverError, SchoolResolver};
pub use statistics::{CanonicalRow, MissingReason, ReportModel};
pub use years::{SUPPORTED_YEARS, dashboard_year_id};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const METHOD_VERSION: &str = "2026-07-descriptive-v2";
