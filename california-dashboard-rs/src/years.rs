//! Authoritative Dashboard year identifiers and endpoint construction.

use thiserror::Error;

pub const SUPPORTED_YEARS: [u16; 9] = [2017, 2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025];
pub const DASHBOARD_BASE_URL: &str = "https://api.caschooldashboard.org/Reports";

/// Maps a calendar year to the exact School Dashboard API identifier.
pub const fn dashboard_year_id(year: u16) -> Option<u8> {
    match year {
        2017 => Some(3),
        2018 => Some(4),
        2019 => Some(5),
        2020 => Some(6),
        2021 => Some(7),
        2022 => Some(8),
        2023 => Some(9),
        2024 => Some(10),
        2025 => Some(11),
        _ => None,
    }
}

pub const fn is_supported_year(year: u16) -> bool {
    dashboard_year_id(year).is_some()
}

/// Constructs the documented SummaryCards URL for one school and year.
pub fn summary_cards_url(cds_code: &str, year: u16) -> Result<String, YearError> {
    if !is_valid_cds_code(cds_code) {
        return Err(YearError::InvalidCdsCode(cds_code.to_owned()));
    }
    let year_id = dashboard_year_id(year).ok_or(YearError::UnsupportedYear(year))?;
    Ok(format!(
        "{DASHBOARD_BASE_URL}/{cds_code}/{year_id}/SummaryCards"
    ))
}

pub fn is_valid_cds_code(cds_code: &str) -> bool {
    cds_code.len() == 14 && cds_code.bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum YearError {
    #[error("unsupported Dashboard year {0}; supported years are 2017 through 2025")]
    UnsupportedYear(u16),
    #[error("invalid CDS code {0:?}; a CDS code must contain exactly 14 digits")]
    InvalidCdsCode(String),
}
