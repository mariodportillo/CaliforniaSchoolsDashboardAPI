import csv
import sqlite3

CSV_FILE = "pubschls.csv"
DB_FILE  = "pubschls.db"
TABLE    = "schools"

COLUMN_TYPES = {
    "CDSCode":              "TEXT",
    "NCESDist":             "TEXT",
    "NCESSchool":           "TEXT",
    "StatusType":           "TEXT",
    "County":               "TEXT",
    "District":             "TEXT",
    "School":               "TEXT",
    "Street":               "TEXT",
    "StreetAbr":            "TEXT",
    "City":                 "TEXT",
    "Zip":                  "TEXT",
    "State":                "TEXT",
    "MailStreet":           "TEXT",
    "MailStrAbr":           "TEXT",
    "MailCity":             "TEXT",
    "MailZip":              "TEXT",
    "MailState":            "TEXT",
    "Phone":                "TEXT",
    "Ext":                  "TEXT",
    "FaxNumber":            "TEXT",
    "WebSite":              "TEXT",
    "OpenDate":             "TEXT",
    "ClosedDate":           "TEXT",
    "Charter":              "TEXT",
    "CharterNum":           "TEXT",
    "FundingType":          "TEXT",
    "DOC":                  "TEXT",
    "DOCType":              "TEXT",
    "SOC":                  "TEXT",
    "SOCType":              "TEXT",
    "EdOpsCode":            "TEXT",
    "EdOpsName":            "TEXT",
    "EILCode":              "TEXT",
    "EILName":              "TEXT",
    "GSoffered":            "TEXT",
    "GSserved":             "TEXT",
    "Virtual":              "TEXT",
    "Magnet":               "TEXT",
    "YearRoundYN":          "TEXT",
    "FederalDFCDistrictID": "TEXT",
    "Latitude":             "REAL",
    "Longitude":            "REAL",
    "AdmFName":             "TEXT",
    "AdmLName":             "TEXT",
    "LastUpDate":           "TEXT",
    "Multilingual":         "TEXT",
}

NULL_SENTINELS = {"No Data", "N/A", "", "NULL"}

def clean(value: str):
    return None if value.strip() in NULL_SENTINELS else value.strip()

def main():
    conn = sqlite3.connect(DB_FILE)
    cur  = conn.cursor()

    headers = list(COLUMN_TYPES.keys())
    cols    = ", ".join(f'"{h}" {COLUMN_TYPES[h]}' for h in headers)

    cur.execute(f'DROP TABLE IF EXISTS "{TABLE}"')
    cur.execute(f'CREATE TABLE "{TABLE}" ({cols})')

    cur.execute(f'CREATE INDEX idx_county   ON "{TABLE}" (County)')
    cur.execute(f'CREATE INDEX idx_district ON "{TABLE}" (District)')
    cur.execute(f'CREATE INDEX idx_status   ON "{TABLE}" (StatusType)')
    cur.execute(f'CREATE INDEX idx_cds      ON "{TABLE}" (CDSCode)')

    placeholders = ", ".join("?" for _ in headers)
    row_count = 0

    with open(CSV_FILE, newline="", encoding="utf-8-sig") as f:
        reader = csv.DictReader(f)
        for row in reader:
            values = [clean(row.get(h, "")) for h in headers]
            cur.execute(f'INSERT INTO "{TABLE}" VALUES ({placeholders})', values)
            row_count += 1

    conn.commit()
    conn.close()
    print(f"Done! {row_count} rows inserted into '{DB_FILE}'")

if __name__ == "__main__":
    main()
