//! Import and validation tools for the CDE public-school directory.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use csv::StringRecord;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, params_from_iter};
use serde::Serialize;
use thiserror::Error;

pub const SCHOOL_COLUMNS: [&str; 46] = [
    "CDSCode",
    "NCESDist",
    "NCESSchool",
    "StatusType",
    "County",
    "District",
    "School",
    "Street",
    "StreetAbr",
    "City",
    "Zip",
    "State",
    "MailStreet",
    "MailStrAbr",
    "MailCity",
    "MailZip",
    "MailState",
    "Phone",
    "Ext",
    "FaxNumber",
    "WebSite",
    "OpenDate",
    "ClosedDate",
    "Charter",
    "CharterNum",
    "FundingType",
    "DOC",
    "DOCType",
    "SOC",
    "SOCType",
    "EdOpsCode",
    "EdOpsName",
    "EILCode",
    "EILName",
    "GSoffered",
    "GSserved",
    "Virtual",
    "Magnet",
    "YearRoundYN",
    "FederalDFCDistrictID",
    "Latitude",
    "Longitude",
    "AdmFName",
    "AdmLName",
    "LastUpDate",
    "Multilingual",
];

const REAL_COLUMNS: [&str; 2] = ["Latitude", "Longitude"];
const NULL_SENTINELS: [&str; 4] = ["No Data", "N/A", "", "NULL"];

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid real value {value:?} in {column} at CSV row {row}")]
    InvalidReal {
        column: String,
        value: String,
        row: usize,
    },
    #[error("the school database does not contain the expected schools table")]
    MissingTable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImportSummary {
    pub rows_inserted: usize,
    pub null_values: usize,
    pub duplicate_cds_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FieldDifference {
    pub column: String,
    pub csv_value: Option<String>,
    pub database_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RowDifference {
    pub cds_code: String,
    pub fields: Vec<FieldDifference>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DatabaseValidation {
    pub csv_row_count: usize,
    pub database_row_count: usize,
    pub only_in_csv: Vec<String>,
    pub only_in_database: Vec<String>,
    pub changed_rows: Vec<RowDifference>,
    pub duplicate_csv_cds_codes: Vec<String>,
    pub duplicate_database_cds_codes: Vec<String>,
}

impl DatabaseValidation {
    pub fn is_exact_match(&self) -> bool {
        self.only_in_csv.is_empty()
            && self.only_in_database.is_empty()
            && self.changed_rows.is_empty()
            && self.duplicate_csv_cds_codes.is_empty()
            && self.duplicate_database_cds_codes.is_empty()
    }

    pub fn issue_count(&self) -> usize {
        self.only_in_csv.len()
            + self.only_in_database.len()
            + self.changed_rows.len()
            + self.duplicate_csv_cds_codes.len()
            + self.duplicate_database_cds_codes.len()
    }
}

/// Replace the `schools` table with the contents of a CDE `pubschls.csv` file.
///
/// The replacement is transactional: malformed input leaves the prior table
/// untouched. The four null sentinels and column affinities match the original
/// Python importer.
pub fn import_school_csv(
    csv_path: impl AsRef<Path>,
    database_path: impl AsRef<Path>,
) -> Result<ImportSummary, DatabaseError> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(csv_path)?;
    let source_headers = reader.headers()?.clone();
    let header_index = header_indices(&source_headers);

    let mut connection = Connection::open(database_path)?;
    let transaction = connection.transaction()?;
    transaction.execute_batch(&schema_sql())?;

    let placeholders = (1..=SCHOOL_COLUMNS.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let insert_sql = format!("INSERT INTO schools VALUES ({placeholders})");
    let mut statement = transaction.prepare(&insert_sql)?;

    let mut row_count = 0usize;
    let mut null_values = 0usize;
    let mut seen_cds = BTreeSet::new();
    let mut duplicate_cds = BTreeSet::new();

    for (offset, row) in reader.records().enumerate() {
        let row = row?;
        let mut values = Vec::with_capacity(SCHOOL_COLUMNS.len());
        for column in SCHOOL_COLUMNS {
            let raw = header_index
                .get(column)
                .and_then(|index| row.get(*index))
                .unwrap_or("");
            let cleaned = clean(raw);
            if cleaned.is_none() {
                null_values += 1;
            }

            if REAL_COLUMNS.contains(&column) {
                let value = match cleaned {
                    Some(text) => Value::Real(text.parse::<f64>().map_err(|_| {
                        DatabaseError::InvalidReal {
                            column: column.to_owned(),
                            value: text.to_owned(),
                            row: offset + 2,
                        }
                    })?),
                    None => Value::Null,
                };
                values.push(value);
            } else {
                values.push(cleaned.map_or(Value::Null, |value| Value::Text(value.to_owned())));
            }
        }

        if let Some(Value::Text(cds)) = values.first()
            && !seen_cds.insert(cds.clone())
        {
            duplicate_cds.insert(cds.clone());
        }

        statement.execute(params_from_iter(values.iter()))?;
        row_count += 1;
    }

    drop(statement);
    transaction.commit()?;

    Ok(ImportSummary {
        rows_inserted: row_count,
        null_values,
        duplicate_cds_codes: duplicate_cds.into_iter().collect(),
    })
}

/// Compare the CSV source to the SQLite cache field by field.
pub fn validate_school_database(
    csv_path: impl AsRef<Path>,
    database_path: impl AsRef<Path>,
) -> Result<DatabaseValidation, DatabaseError> {
    let (csv_rows, csv_count, duplicate_csv) = load_csv_rows(csv_path.as_ref())?;
    let connection =
        Connection::open_with_flags(database_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let (database_rows, database_count, duplicate_database) = load_database_rows(&connection)?;

    let csv_keys = csv_rows.keys().cloned().collect::<BTreeSet<_>>();
    let database_keys = database_rows.keys().cloned().collect::<BTreeSet<_>>();

    let only_in_csv = csv_keys.difference(&database_keys).cloned().collect();
    let only_in_database = database_keys.difference(&csv_keys).cloned().collect();
    let mut changed_rows = Vec::new();

    for cds in csv_keys.intersection(&database_keys) {
        let csv_row = &csv_rows[cds];
        let database_row = &database_rows[cds];
        let mut fields = Vec::new();
        for (index, column) in SCHOOL_COLUMNS.iter().enumerate().skip(1) {
            if !values_equal(
                column,
                csv_row[index].as_deref(),
                database_row[index].as_deref(),
            ) {
                fields.push(FieldDifference {
                    column: (*column).to_owned(),
                    csv_value: csv_row[index].clone(),
                    database_value: database_row[index].clone(),
                });
            }
        }
        if !fields.is_empty() {
            changed_rows.push(RowDifference {
                cds_code: cds.clone(),
                fields,
            });
        }
    }

    Ok(DatabaseValidation {
        csv_row_count: csv_count,
        database_row_count: database_count,
        only_in_csv,
        only_in_database,
        changed_rows,
        duplicate_csv_cds_codes: duplicate_csv,
        duplicate_database_cds_codes: duplicate_database,
    })
}

type RowMap = BTreeMap<String, Vec<Option<String>>>;

fn load_csv_rows(path: &Path) -> Result<(RowMap, usize, Vec<String>), DatabaseError> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_path(path)?;
    let headers = reader.headers()?.clone();
    let indices = header_indices(&headers);
    let mut rows = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    let mut physical_count = 0;

    for row in reader.records() {
        let row = row?;
        physical_count += 1;
        let values = SCHOOL_COLUMNS
            .iter()
            .map(|column| {
                indices
                    .get(*column)
                    .and_then(|index| row.get(*index))
                    .and_then(clean)
                    .map(ToOwned::to_owned)
            })
            .collect::<Vec<_>>();
        let Some(cds) = values.first().and_then(Clone::clone) else {
            continue;
        };
        if rows.insert(cds.clone(), values).is_some() {
            duplicates.insert(cds);
        }
    }
    Ok((rows, physical_count, duplicates.into_iter().collect()))
}

fn load_database_rows(
    connection: &Connection,
) -> Result<(RowMap, usize, Vec<String>), DatabaseError> {
    let table_exists: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schools'",
        [],
        |row| row.get(0),
    )?;
    if table_exists == 0 {
        return Err(DatabaseError::MissingTable);
    }

    let selected = SCHOOL_COLUMNS
        .iter()
        .map(|column| format!("\"{column}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = connection.prepare(&format!("SELECT {selected} FROM schools"))?;
    let mut cursor = statement.query([])?;
    let mut rows = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    let mut physical_count = 0;

    while let Some(row) = cursor.next()? {
        physical_count += 1;
        let mut values = Vec::with_capacity(SCHOOL_COLUMNS.len());
        for index in 0..SCHOOL_COLUMNS.len() {
            let value = match row.get_ref(index)? {
                ValueRef::Null => None,
                ValueRef::Integer(value) => Some(value.to_string()),
                ValueRef::Real(value) => Some(value.to_string()),
                ValueRef::Text(value) => Some(String::from_utf8_lossy(value).into_owned()),
                ValueRef::Blob(value) => Some(String::from_utf8_lossy(value).into_owned()),
            };
            values.push(value);
        }
        let Some(cds) = values.first().and_then(Clone::clone) else {
            continue;
        };
        if rows.insert(cds.clone(), values).is_some() {
            duplicates.insert(cds);
        }
    }

    Ok((rows, physical_count, duplicates.into_iter().collect()))
}

fn clean(value: &str) -> Option<&str> {
    let value = value.trim();
    (!NULL_SENTINELS.contains(&value)).then_some(value)
}

fn header_indices(headers: &StringRecord) -> HashMap<String, usize> {
    headers
        .iter()
        .enumerate()
        .map(|(index, header)| (header.trim_start_matches('\u{feff}').to_owned(), index))
        .collect()
}

fn values_equal(column: &str, left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) if REAL_COLUMNS.contains(&column) => {
            match (left.parse::<f64>(), right.parse::<f64>()) {
                (Ok(left), Ok(right)) => (left - right).abs() < 1e-9,
                _ => left == right,
            }
        }
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn schema_sql() -> String {
    let columns = SCHOOL_COLUMNS
        .iter()
        .map(|column| {
            let affinity = if REAL_COLUMNS.contains(column) {
                "REAL"
            } else {
                "TEXT"
            };
            format!("\"{column}\" {affinity}")
        })
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "DROP TABLE IF EXISTS schools;\n\
         CREATE TABLE schools ({columns});\n\
         CREATE INDEX idx_county ON schools (County);\n\
         CREATE INDEX idx_district ON schools (District);\n\
         CREATE INDEX idx_status ON schools (StatusType);\n\
         CREATE INDEX idx_cds ON schools (CDSCode);"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn cleans_only_the_documented_null_sentinels() {
        assert_eq!(clean(" No Data "), None);
        assert_eq!(clean("N/A"), None);
        assert_eq!(clean(""), None);
        assert_eq!(clean("NULL"), None);
        assert_eq!(clean("null"), Some("null"));
        assert_eq!(clean("0"), Some("0"));
    }

    #[test]
    fn compares_real_values_with_legacy_tolerance() {
        assert!(values_equal("Latitude", Some("1"), Some("1.0000000005")));
        assert!(!values_equal("Latitude", Some("1"), Some("1.000000002")));
        assert!(!values_equal("School", Some("1"), Some("1.0")));
    }

    #[test]
    fn imports_and_validates_a_bom_csv_without_losing_leading_zeroes() {
        let temp = tempdir().expect("temporary directory");
        let csv_path = temp.path().join("pubschls.csv");
        let database_path = temp.path().join("pubschls.db");

        let mut first = vec!["No Data".to_owned(); SCHOOL_COLUMNS.len()];
        first[0] = "01100170000000".to_owned();
        first[3] = "Active".to_owned();
        first[4] = "Alameda".to_owned();
        first[5] = "Example District".to_owned();
        first[6] = "Example School".to_owned();
        first[40] = "37.5".to_owned();
        first[41] = "-122.1".to_owned();

        let csv = format!(
            "\u{feff}{}\n{}\n",
            SCHOOL_COLUMNS.join(","),
            first.join(",")
        );
        fs::write(&csv_path, csv).expect("write fixture");

        let imported = import_school_csv(&csv_path, &database_path).expect("import succeeds");
        assert_eq!(imported.rows_inserted, 1);
        assert!(imported.duplicate_cds_codes.is_empty());

        let connection = Connection::open(&database_path).expect("open database");
        let cds: String = connection
            .query_row("SELECT CDSCode FROM schools", [], |row| row.get(0))
            .expect("read code");
        assert_eq!(cds, "01100170000000");

        let validation =
            validate_school_database(&csv_path, &database_path).expect("validate succeeds");
        assert!(validation.is_exact_match(), "{validation:#?}");
        assert_eq!(validation.csv_row_count, 1);
        assert_eq!(validation.database_row_count, 1);
    }

    #[test]
    fn malformed_real_rolls_back_the_table_replacement() {
        let temp = tempdir().expect("temporary directory");
        let csv_path = temp.path().join("invalid.csv");
        let database_path = temp.path().join("pubschls.db");
        let connection = Connection::open(&database_path).expect("open database");
        connection
            .execute_batch(
                "CREATE TABLE schools (marker TEXT); INSERT INTO schools VALUES ('old');",
            )
            .expect("seed prior table");
        drop(connection);

        let mut row = vec!["No Data".to_owned(); SCHOOL_COLUMNS.len()];
        row[0] = "01100170000000".to_owned();
        row[40] = "not-a-number".to_owned();
        fs::write(
            &csv_path,
            format!("{}\n{}\n", SCHOOL_COLUMNS.join(","), row.join(",")),
        )
        .expect("write fixture");

        let error = import_school_csv(&csv_path, &database_path).expect_err("import fails");
        assert!(matches!(error, DatabaseError::InvalidReal { .. }));

        let connection = Connection::open(&database_path).expect("reopen database");
        let marker: String = connection
            .query_row("SELECT marker FROM schools", [], |row| row.get(0))
            .expect("prior table survives");
        assert_eq!(marker, "old");
    }
}
