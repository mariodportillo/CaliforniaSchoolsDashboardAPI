# California Dashboard (Rust)

A Rust rewrite of the C++ California School Dashboard client, plus a local web
interface for pulling data and generating statistically grounded reports in CSV,
Excel, HTML, and PDF.

It preserves the documented behavior of the original project while correcting
defects that could return the wrong school or turn missing data into a numeric
zero. See [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md) for the full
compatibility contract and [`docs/STATISTICAL_METHODS.md`](docs/STATISTICAL_METHODS.md)
for the reporting policy.

## What it does

1. Loads active CA public schools from the existing `pubschls.db` SQLite cache.
2. Resolves names to CDS codes (exact → substring → Levenshtein), or takes a CDS
   code directly so a school selected in the UI is never re-matched.
3. Builds `https://api.caschooldashboard.org/Reports/{CDS}/{year-id}/SummaryCards`
   URLs for each (school, year) pair.
4. Fetches concurrently with a token-bucket rate limiter and bounded retries.
5. Parses each response into typed indicators where missing values stay missing.
   The exact raw card and primary/secondary JSON blocks are retained for the
   same save/load/inspection workflow exposed by the C++ `SummaryCard` class.
6. Applies one privacy-filtered canonical model (`statistics.rs`) that every
   output — browser, CSV, Excel, HTML, PDF — consumes identically.

## Build

```bash
cargo build --release
cargo test          # unit + integration tests
cargo clippy --all-targets
```

The commands look for `pubschls.db` in the current or parent directory; pass
`--db PATH` to override. The database is opened read-only and is not duplicated
into this project.

## Commands

### Serve the local UI

```bash
cargo run --release -- serve --port 8787
```

Open <http://127.0.0.1:8787>. Search schools, choose reporting years, run the
pull (or select every active school), watch progress, then download the four
report exports. The UI also provides complete school/district directory CSV and
Excel downloads. The server binds loopback by default and serves its assets
from the binary.

### Pull from the command line

```bash
# Specific schools by 14-digit CDS code
cargo run --release -- pull \
  --school-cds 01100170112607 --school-cds 01100170114363 \
  --years 2023,2024,2025 \
  --output dashboard-output --stem ca-report

# Every active school
cargo run --release -- pull --all --years 2025
```

This writes `<stem>.csv`, `.xlsx`, `.html`, and `.pdf` to the output directory.

### Export the complete school and district directory

```bash
cargo run --release -- export-directory \
  --output dashboard-output --stem cde-school-directory
```

This writes the five reference columns—`Record Type`, `CDS Code`, `District`,
`School`, and `Status`—to CSV and to a filterable Excel table with a formula
total row. Unlike Dashboard pulls, the directory includes inactive records.

### Maintain the school cache

```bash
cargo run --release -- import-schools   --csv pubschls.csv --db pubschls.db
cargo run --release -- validate-schools --csv pubschls.csv --db pubschls.db
```

## Reports

Every export is descriptive and observational. A missing value stays missing; a
valid zero stays zero. Private measures and measures with a known denominator of
10 or fewer are fully suppressed; denominators of 11–29 are shown with a caution
flag. The report never fabricates confidence intervals, p-values, effect sizes,
rankings, or a composite score, because the SummaryCards do not supply the raw
counts, variances, or independent sample those methods require. Each row carries
its source URL, retrieval time, and method version for provenance.

- **CSV** — the complete canonical row set, one row per school/year/indicator.
- **Excel** — a filterable report-data table plus separate sheets for data
  quality, indicator definitions, the retrieval log, methods, and sources.
- **HTML** — a readable report: completeness summary, methods, limitations, the
  full data table, a row-by-row description, the retrieval log, and sources.
- **PDF** — a print-friendly narrative of the same summary, methods,
  limitations, findings, and sources.

## Load testing

A local harness exercises the fetch scheduler against an in-process mock server
and never contacts the public API:

```bash
cargo run --release --bin stress -- --requests 20000 --concurrency 50 \
  --requests-per-second 1000 --delay-ms 15
```

It asserts that every request completes, result order is preserved, and the
configured concurrency ceiling is never exceeded.

## Layout

```
src/
  resolver.rs     School lookup, matching, URL building
  client.rs       Concurrent fetch engine with rate limiting and retries
  model.rs        Typed SummaryCards response and provenance
  statistics.rs   Privacy-filtered canonical report model
  export.rs       CSV / Excel / HTML / PDF rendering
  web.rs          Local Axum UI and job API
  database.rs     CSV → SQLite import and validation
  years.rs        Supported year ↔ Dashboard ID mapping
  bin/stress.rs   Local load harness
```

## License

MIT
