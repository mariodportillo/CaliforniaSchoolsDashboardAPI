"""
validate_db.py
Compares pubschls.csv against pubschls.db and reports differences.

Usage:
    python3 validate_db.py
    python3 validate_db.py --csv path/to/pubschls.csv --db path/to/pubschls.db
    python3 validate_db.py --summary   # print summary only, no per-row diffs
"""

import csv
import sqlite3
import argparse
from dataclasses import dataclass, field
from typing import Optional

# =============================================================================
# Config
# =============================================================================

CSV_FILE  = "pubschls.csv"
DB_FILE   = "pubschls.db"
TABLE     = "schools"
PK        = "CDSCode"

NULL_SENTINELS = {"No Data", "N/A", "", "NULL"}

COLUMNS = [
    "CDSCode", "NCESDist", "NCESSchool", "StatusType", "County", "District",
    "School", "Street", "StreetAbr", "City", "Zip", "State", "MailStreet",
    "MailStrAbr", "MailCity", "MailZip", "MailState", "Phone", "Ext",
    "FaxNumber", "WebSite", "OpenDate", "ClosedDate", "Charter", "CharterNum",
    "FundingType", "DOC", "DOCType", "SOC", "SOCType", "EdOpsCode", "EdOpsName",
    "EILCode", "EILName", "GSoffered", "GSserved", "Virtual", "Magnet",
    "YearRoundYN", "FederalDFCDistrictID", "Latitude", "Longitude",
    "AdmFName", "AdmLName", "LastUpDate", "Multilingual",
]

REAL_COLS = {"Latitude", "Longitude"}

# =============================================================================
# Data classes
# =============================================================================

@dataclass
class FieldDiff:
    column:    str
    csv_value: Optional[str]
    db_value:  Optional[str]

@dataclass
class RowDiff:
    cds_code: str
    diffs:    list[FieldDiff] = field(default_factory=list)

@dataclass
class Report:
    only_in_csv:    list[str]       = field(default_factory=list)
    only_in_db:     list[str]       = field(default_factory=list)
    changed_rows:   list[RowDiff]   = field(default_factory=list)
    csv_row_count:  int = 0
    db_row_count:   int = 0

# =============================================================================
# Helpers
# =============================================================================

def clean(value: str) -> Optional[str]:
    v = value.strip()
    return None if v in NULL_SENTINELS else v

def values_equal(col: str, csv_val: Optional[str], db_val) -> bool:
    if csv_val is None and db_val is None:
        return True
    if csv_val is None or db_val is None:
        return False
    if col in REAL_COLS:
        try:
            return abs(float(csv_val) - float(db_val)) < 1e-9
        except ValueError:
            return str(csv_val) == str(db_val)
    return csv_val == str(db_val)

# =============================================================================
# Load data
# =============================================================================

def load_csv(path: str) -> dict[str, dict]:
    rows = {}
    with open(path, newline="", encoding="utf-8-sig") as f:
        reader = csv.DictReader(f)
        for row in reader:
            cds = row.get(PK, "").strip()
            if not cds:
                continue
            rows[cds] = {col: clean(row.get(col, "")) for col in COLUMNS}
    return rows

def load_db(path: str) -> dict[str, dict]:
    conn = sqlite3.connect(path)
    conn.row_factory = sqlite3.Row
    cur  = conn.cursor()
    cur.execute(f"SELECT {', '.join(COLUMNS)} FROM {TABLE}")
    rows = {}
    for row in cur.fetchall():
        cds = row[PK]
        if not cds:
            continue
        rows[cds] = {col: row[col] for col in COLUMNS}
    conn.close()
    return rows

# =============================================================================
# Compare
# =============================================================================

def compare(csv_rows: dict, db_rows: dict) -> Report:
    report = Report(
        csv_row_count=len(csv_rows),
        db_row_count=len(db_rows),
    )

    csv_keys = set(csv_rows.keys())
    db_keys  = set(db_rows.keys())

    report.only_in_csv = sorted(csv_keys - db_keys)
    report.only_in_db  = sorted(db_keys  - csv_keys)

    for cds in sorted(csv_keys & db_keys):
        csv_row = csv_rows[cds]
        db_row  = db_rows[cds]
        diffs   = []

        for col in COLUMNS:
            if col == PK:
                continue
            csv_val = csv_row.get(col)
            db_val  = db_row.get(col)
            if not values_equal(col, csv_val, db_val):
                diffs.append(FieldDiff(
                    column    = col,
                    csv_value = csv_val,
                    db_value  = str(db_val) if db_val is not None else None,
                ))

        if diffs:
            report.changed_rows.append(RowDiff(cds_code=cds, diffs=diffs))

    return report

# =============================================================================
# Print report
# =============================================================================

def print_report(report: Report, summary_only: bool) -> None:
    print("=" * 70)
    print("  pubschls.csv  vs  pubschls.db — Validation Report")
    print("=" * 70)
    print(f"  CSV rows : {report.csv_row_count:,}")
    print(f"  DB rows  : {report.db_row_count:,}")
    print()

    if report.only_in_csv:
        print(f"[MISSING FROM DB]  {len(report.only_in_csv)} row(s) in CSV but not in DB:")
        for cds in report.only_in_csv[:20]:
            print(f"    {cds}")
        if len(report.only_in_csv) > 20:
            print(f"    ... and {len(report.only_in_csv) - 20} more")
        print()
    else:
        print("[OK] No rows missing from DB.\n")

    if report.only_in_db:
        print(f"[EXTRA IN DB]  {len(report.only_in_db)} row(s) in DB but not in CSV:")
        for cds in report.only_in_db[:20]:
            print(f"    {cds}")
        if len(report.only_in_db) > 20:
            print(f"    ... and {len(report.only_in_db) - 20} more")
        print()
    else:
        print("[OK] No phantom rows in DB.\n")

    if report.changed_rows:
        print(f"[CHANGED]  {len(report.changed_rows)} row(s) have field differences:")
        if not summary_only:
            for row_diff in report.changed_rows:
                print(f"\n  CDSCode: {row_diff.cds_code}")
                for fd in row_diff.diffs:
                    print(f"    {fd.column:<22}  CSV: {str(fd.csv_value):<40}  DB: {fd.db_value}")
        print()
    else:
        print("[OK] All matching rows are identical.\n")

    total_issues = (len(report.only_in_csv) +
                    len(report.only_in_db)  +
                    len(report.changed_rows))
    print("=" * 70)
    if total_issues == 0:
        print("  ✅  DB is perfectly in sync with CSV.")
    else:
        print(f"  ⚠️   {total_issues} issue(s) found. Re-run csv_to_sqlite.py to fix.")
    print("=" * 70)

# =============================================================================
# Entry point
# =============================================================================

def main():
    parser = argparse.ArgumentParser(description="Validate pubschls.db against pubschls.csv")
    parser.add_argument("--csv",     default=CSV_FILE, help="Path to pubschls.csv")
    parser.add_argument("--db",      default=DB_FILE,  help="Path to pubschls.db")
    parser.add_argument("--summary", action="store_true", help="Print summary only, no per-row diffs")
    args = parser.parse_args()

    print(f"Loading CSV: {args.csv}")
    csv_rows = load_csv(args.csv)

    print(f"Loading DB:  {args.db}\n")
    db_rows = load_db(args.db)

    report = compare(csv_rows, db_rows)
    print_report(report, summary_only=args.summary)

if __name__ == "__main__":
    main()
