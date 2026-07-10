//! Read-only school identity lookup, deterministic matching, and URL planning.

use crate::model::{DirectoryRecord, FetchSpec, SchoolRecord};
use crate::years::{YearError, dashboard_year_id, is_valid_cds_code, summary_cards_url};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;
use thiserror::Error;

const MIN_SUBSTRING_QUERY_CHARS: usize = 5;
const MAX_EDIT_DISTANCE: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    ExactCds,
    ExactName,
    Substring,
    Levenshtein,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchResult {
    pub school: SchoolRecord,
    pub kind: MatchKind,
    /// Present only for a Levenshtein match.
    pub distance: Option<usize>,
}

/// In-memory view of active schools, loaded once from a read-only connection.
#[derive(Debug)]
pub struct SchoolResolver {
    schools: Vec<SchoolRecord>,
    schools_by_cds: BTreeMap<String, SchoolRecord>,
    directory_records: Vec<DirectoryRecord>,
}

impl SchoolResolver {
    /// Opens the CDE cache read-only and loads every active, identified school.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ResolverError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        let mut statement = connection.prepare(
            "SELECT CDSCode, School, County, District, City
             FROM schools
             WHERE StatusType = 'Active'
               AND CDSCode IS NOT NULL
               AND School IS NOT NULL",
        )?;

        let rows = statement.query_map([], |row| {
            Ok(SchoolRecord {
                cds_code: row.get(0)?,
                school_name: row.get(1)?,
                county: row.get(2)?,
                district: row.get(3)?,
                city: row.get(4)?,
            })
        })?;

        let mut schools_by_cds = BTreeMap::new();
        for row in rows {
            let mut school = row?;
            school.cds_code = school.cds_code.trim().to_owned();
            school.school_name = school.school_name.trim().to_owned();
            school.county = clean_optional_text(school.county);
            school.district = clean_optional_text(school.district);
            school.city = clean_optional_text(school.city);

            if !is_valid_cds_code(&school.cds_code) {
                return Err(ResolverError::InvalidDatabaseCdsCode(
                    school.cds_code.clone(),
                ));
            }
            if school.school_name.is_empty() {
                return Err(ResolverError::EmptySchoolName(school.cds_code.clone()));
            }
            if schools_by_cds
                .insert(school.cds_code.clone(), school.clone())
                .is_some()
            {
                return Err(ResolverError::DuplicateCdsCode(school.cds_code));
            }
        }
        drop(statement);

        let mut directory_statement = connection.prepare(
            "SELECT CDSCode, District, School, StatusType
             FROM schools
             WHERE CDSCode IS NOT NULL",
        )?;
        let directory_rows = directory_statement.query_map([], |row| {
            let cds_code: String = row.get(0)?;
            let district: Option<String> = row.get(1)?;
            let school: Option<String> = row.get(2)?;
            let status: Option<String> = row.get(3)?;
            Ok((cds_code, district, school, status))
        })?;
        let mut directory_records = Vec::new();
        for row in directory_rows {
            let (cds_code, district, school, status) = row?;
            let cds_code = cds_code.trim().to_owned();
            if !is_valid_cds_code(&cds_code) {
                // Spreadsheet source files may contain a formula-driven total
                // row such as `Total Records =`; it is layout, not directory data.
                continue;
            }
            let district = district.unwrap_or_default().trim().to_owned();
            let school = school.unwrap_or_default().trim().to_owned();
            directory_records.push(DirectoryRecord {
                record_type: if school.is_empty() {
                    "District".to_owned()
                } else {
                    "School".to_owned()
                },
                cds_code,
                district,
                school: if school.is_empty() {
                    "No Data".to_owned()
                } else {
                    school
                },
                status: status.unwrap_or_default().trim().to_owned(),
            });
        }
        drop(directory_statement);
        drop(connection);

        let mut schools: Vec<_> = schools_by_cds.values().cloned().collect();
        schools.sort_by(compare_schools);
        directory_records.sort_by(compare_directory_records);
        Ok(Self {
            schools,
            schools_by_cds,
            directory_records,
        })
    }

    pub fn schools(&self) -> &[SchoolRecord] {
        &self.schools
    }

    pub fn all_schools(&self) -> &[SchoolRecord] {
        self.schools()
    }

    /// Every school and district row, including inactive records, for CDE-style exports.
    pub fn directory_records(&self) -> &[DirectoryRecord] {
        &self.directory_records
    }

    /// C++ `buildAllSchoolsMap(years)` equivalent with duplicate names
    /// disambiguated as `School name (CDS)`.
    pub fn build_all_schools_map(
        &self,
        years: &[u16],
    ) -> Result<BTreeMap<String, Vec<u16>>, ResolverError> {
        validate_years(years)?;
        let mut name_counts = BTreeMap::<&str, usize>::new();
        for school in &self.schools {
            *name_counts.entry(&school.school_name).or_default() += 1;
        }
        Ok(self
            .schools
            .iter()
            .map(|school| {
                let name = if name_counts.get(school.school_name.as_str()) == Some(&1) {
                    school.school_name.clone()
                } else {
                    format!("{} ({})", school.school_name, school.cds_code)
                };
                (name, years.to_vec())
            })
            .collect())
    }

    /// Resolves the same name/CDS/fuzzy inputs accepted by the original C++
    /// resolver, then builds stable fetch specifications.
    pub fn fetch_specs_for_queries(
        &self,
        queries: &[String],
        years: &[u16],
    ) -> Result<Vec<FetchSpec>, ResolverError> {
        validate_years(years)?;
        let mut specs = Vec::with_capacity(queries.len().saturating_mul(years.len()));
        for query in queries {
            let school = self.resolve(query)?.school;
            for &year in years {
                specs.push(self.fetch_spec(&school.cds_code, year)?);
            }
        }
        Ok(specs)
    }

    pub fn len(&self) -> usize {
        self.schools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.schools.is_empty()
    }

    /// Selects a school by its unique 14-digit CDS code; no fuzzy matching.
    pub fn school_by_cds(&self, cds_code: &str) -> Option<&SchoolRecord> {
        self.schools_by_cds.get(cds_code.trim())
    }

    /// Searches school name, county, district, city, and CDS code.
    ///
    /// Results are relevance-ranked and then deterministically ordered. A
    /// limit of zero intentionally returns no results.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SchoolRecord> {
        if limit == 0 {
            return Vec::new();
        }
        let query = normalize(query);
        if query.is_empty() {
            return self.schools.iter().take(limit).cloned().collect();
        }

        let mut matches: Vec<(u8, &SchoolRecord)> = self
            .schools
            .iter()
            .filter_map(|school| search_rank(school, &query).map(|rank| (rank, school)))
            .collect();
        matches.sort_by(|(left_rank, left), (right_rank, right)| {
            left_rank
                .cmp(right_rank)
                .then_with(|| compare_schools(left, right))
        });
        matches
            .into_iter()
            .take(limit)
            .map(|(_, school)| school.clone())
            .collect()
    }

    /// Resolves a CDS code or school name using the documented three tiers.
    ///
    /// Ties are returned as explicit ambiguity errors instead of depending on
    /// SQLite row order or hash-map iteration order.
    pub fn resolve(&self, query: &str) -> Result<MatchResult, ResolverError> {
        let trimmed = query.trim();
        if let Some((name, cds_code)) = split_disambiguated_name(trimmed)
            && let Some(school) = self.school_by_cds(cds_code)
            && normalize(name) == normalize(&school.school_name)
        {
            return Ok(MatchResult {
                school: school.clone(),
                kind: MatchKind::ExactCds,
                distance: None,
            });
        }
        if is_valid_cds_code(trimmed) {
            return self
                .school_by_cds(trimmed)
                .cloned()
                .map(|school| MatchResult {
                    school,
                    kind: MatchKind::ExactCds,
                    distance: None,
                })
                .ok_or_else(|| ResolverError::CdsNotFound(trimmed.to_owned()));
        }

        let query_normalized = normalize(trimmed);
        if query_normalized.is_empty() {
            return Err(ResolverError::NotFound(query.to_owned()));
        }

        let exact: Vec<_> = self
            .schools
            .iter()
            .filter(|school| normalize(&school.school_name) == query_normalized)
            .collect();
        if let Some(school) = unique_or_ambiguous(query, exact)? {
            return Ok(MatchResult {
                school,
                kind: MatchKind::ExactName,
                distance: None,
            });
        }

        if query_normalized.chars().count() >= MIN_SUBSTRING_QUERY_CHARS {
            let mut substring: Vec<(usize, &SchoolRecord)> = self
                .schools
                .iter()
                .filter_map(|school| {
                    let candidate = normalize(&school.school_name);
                    let overlap = if candidate.contains(&query_normalized) {
                        query_normalized.chars().count()
                    } else if query_normalized.contains(&candidate) {
                        candidate.chars().count()
                    } else {
                        0
                    };
                    (overlap >= MIN_SUBSTRING_QUERY_CHARS).then_some((overlap, school))
                })
                .collect();

            if let Some(best_overlap) = substring.iter().map(|(score, _)| *score).max() {
                let best: Vec<_> = substring
                    .drain(..)
                    .filter_map(|(score, school)| (score == best_overlap).then_some(school))
                    .collect();
                if let Some(school) = unique_or_ambiguous(query, best)? {
                    return Ok(MatchResult {
                        school,
                        kind: MatchKind::Substring,
                        distance: None,
                    });
                }
            }
        }

        let mut fuzzy: Vec<(usize, &SchoolRecord)> = self
            .schools
            .iter()
            .map(|school| {
                (
                    levenshtein(&query_normalized, &normalize(&school.school_name)),
                    school,
                )
            })
            .filter(|(distance, _)| *distance <= MAX_EDIT_DISTANCE)
            .collect();
        if let Some(best_distance) = fuzzy.iter().map(|(distance, _)| *distance).min() {
            let best: Vec<_> = fuzzy
                .drain(..)
                .filter_map(|(distance, school)| (distance == best_distance).then_some(school))
                .collect();
            if let Some(school) = unique_or_ambiguous(query, best)? {
                return Ok(MatchResult {
                    school,
                    kind: MatchKind::Levenshtein,
                    distance: Some(best_distance),
                });
            }
        }

        Err(ResolverError::NotFound(query.to_owned()))
    }

    pub fn fetch_spec(&self, cds_code: &str, year: u16) -> Result<FetchSpec, ResolverError> {
        if !is_valid_cds_code(cds_code) {
            return Err(ResolverError::InvalidCdsCode(cds_code.to_owned()));
        }
        let school = self
            .school_by_cds(cds_code)
            .cloned()
            .ok_or_else(|| ResolverError::CdsNotFound(cds_code.to_owned()))?;
        let dashboard_year_id =
            dashboard_year_id(year).ok_or(ResolverError::UnsupportedYear(year))?;
        let url = summary_cards_url(&school.cds_code, year)?;
        Ok(FetchSpec {
            school,
            year,
            dashboard_year_id,
            url,
        })
    }

    /// Builds requests in caller-provided CDS order, then year order.
    pub fn fetch_specs(
        &self,
        cds_codes: &[String],
        years: &[u16],
    ) -> Result<Vec<FetchSpec>, ResolverError> {
        validate_years(years)?;
        let mut specs = Vec::with_capacity(cds_codes.len().saturating_mul(years.len()));
        for cds_code in cds_codes {
            for &year in years {
                specs.push(self.fetch_spec(cds_code, year)?);
            }
        }
        Ok(specs)
    }

    /// Builds requests for all active schools in stable school/year order.
    pub fn all_fetch_specs(&self, years: &[u16]) -> Result<Vec<FetchSpec>, ResolverError> {
        validate_years(years)?;
        let mut specs = Vec::with_capacity(self.schools.len().saturating_mul(years.len()));
        for school in &self.schools {
            for &year in years {
                specs.push(self.fetch_spec(&school.cds_code, year)?);
            }
        }
        Ok(specs)
    }
}

fn split_disambiguated_name(value: &str) -> Option<(&str, &str)> {
    let without_close = value.strip_suffix(')')?;
    let (name, cds_code) = without_close.rsplit_once(" (")?;
    is_valid_cds_code(cds_code).then_some((name, cds_code))
}

fn validate_years(years: &[u16]) -> Result<(), ResolverError> {
    for &year in years {
        if dashboard_year_id(year).is_none() {
            return Err(ResolverError::UnsupportedYear(year));
        }
    }
    Ok(())
}

fn unique_or_ambiguous(
    query: &str,
    mut candidates: Vec<&SchoolRecord>,
) -> Result<Option<SchoolRecord>, ResolverError> {
    candidates.sort_by(|left, right| compare_schools(left, right));
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.first().map(|school| (*school).clone())),
        _ => Err(ResolverError::Ambiguous {
            query: query.to_owned(),
            candidates: candidates.into_iter().cloned().collect(),
        }),
    }
}

fn compare_schools(left: &SchoolRecord, right: &SchoolRecord) -> Ordering {
    normalize(&left.school_name)
        .cmp(&normalize(&right.school_name))
        .then_with(|| left.cds_code.cmp(&right.cds_code))
}

fn compare_directory_records(left: &DirectoryRecord, right: &DirectoryRecord) -> Ordering {
    let left_rank = usize::from(left.record_type != "School");
    let right_rank = usize::from(right.record_type != "School");
    left_rank
        .cmp(&right_rank)
        .then_with(|| normalize(&left.district).cmp(&normalize(&right.district)))
        .then_with(|| normalize(&left.school).cmp(&normalize(&right.school)))
        .then_with(|| left.cds_code.cmp(&right.cds_code))
}

fn search_rank(school: &SchoolRecord, query: &str) -> Option<u8> {
    let name = normalize(&school.school_name);
    let county = school.county.as_deref().map(normalize);
    let district = school.district.as_deref().map(normalize);
    let city = school.city.as_deref().map(normalize);

    if school.cds_code == query {
        Some(0)
    } else if name == query {
        Some(1)
    } else if name.starts_with(query) {
        Some(2)
    } else if name.contains(query) {
        Some(3)
    } else if district.as_deref().is_some_and(|text| text.contains(query)) {
        Some(4)
    } else if county.as_deref().is_some_and(|text| text.contains(query)) {
        Some(5)
    } else if city.as_deref().is_some_and(|text| text.contains(query)) {
        Some(6)
    } else if school.cds_code.contains(query) {
        Some(7)
    } else {
        None
    }
}

fn clean_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Unicode-scalar Levenshtein distance with O(min(m, n)) memory.
fn levenshtein(left: &str, right: &str) -> usize {
    let mut left: Vec<char> = left.chars().collect();
    let mut right: Vec<char> = right.chars().collect();
    if left.len() > right.len() {
        std::mem::swap(&mut left, &mut right);
    }

    let mut previous: Vec<usize> = (0..=left.len()).collect();
    let mut current = vec![0; left.len() + 1];
    for (right_index, right_char) in right.iter().enumerate() {
        current[0] = right_index + 1;
        for (left_index, left_char) in left.iter().enumerate() {
            let insertion = current[left_index] + 1;
            let deletion = previous[left_index + 1] + 1;
            let substitution = previous[left_index] + usize::from(left_char != right_char);
            current[left_index + 1] = insertion.min(deletion).min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[left.len()]
}

#[derive(Debug, Error)]
pub enum ResolverError {
    #[error("failed to read school database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("database contains invalid CDS code {0:?}")]
    InvalidDatabaseCdsCode(String),
    #[error("database contains duplicate CDS code {0}")]
    DuplicateCdsCode(String),
    #[error("database school {0} has an empty name")]
    EmptySchoolName(String),
    #[error("invalid CDS code {0:?}; expected exactly 14 digits")]
    InvalidCdsCode(String),
    #[error("CDS code {0} is not an active school in the database")]
    CdsNotFound(String),
    #[error("no active school matched {0:?}")]
    NotFound(String),
    #[error("school query {query:?} is ambiguous")]
    Ambiguous {
        query: String,
        candidates: Vec<SchoolRecord>,
    },
    #[error("unsupported Dashboard year {0}; supported years are 2017 through 2025")]
    UnsupportedYear(u16),
    #[error(transparent)]
    Year(#[from] YearError),
}

#[cfg(test)]
mod tests {
    use super::levenshtein;

    #[test]
    fn levenshtein_handles_empty_and_unicode_input() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("cafe", "café"), 1);
    }
}
