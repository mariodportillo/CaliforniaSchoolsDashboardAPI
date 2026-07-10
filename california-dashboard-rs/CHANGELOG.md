# Changelog

All notable changes to the Rust California School Dashboard client are recorded
here. This project adheres to [Semantic Versioning](https://semver.org/).

## [1.0.0] — 2026-07-10

Initial Rust rewrite of the C++ California School Dashboard client, adding a
local web UI and statistically grounded report generation (CSV, Excel, HTML,
PDF).

### Added

- **Fetch engine** (`client.rs`) — async concurrent pulls with a token-bucket
  rate limiter, bounded retries (4 attempts, exponential backoff, HTTP
  408/429/5xx + transport errors), per-attempt timeouts, and a payload-size
  cap. Rate and concurrency limits also cover retries.
- **Typed model** (`model.rs`) — SummaryCards parsing for both the array and
  single-object forms, with the exact raw card and primary/secondary JSON
  retained for the same save/load/inspection workflow as the C++ `SummaryCard`.
- **School resolver** (`resolver.rs`) — exact / substring / Levenshtein
  matching, deterministic search, and a full CDE-style directory (including
  inactive records).
- **Statistical model** (`statistics.rs`) — a single privacy-filtered canonical
  row set consumed identically by every output. Implements the reporting policy
  in `docs/STATISTICAL_METHODS.md`.
- **Exports** (`export.rs`) — CSV, a multi-sheet Excel workbook, a self-contained
  HTML report, and a dependency-free PDF report. Includes complete school /
  district directory CSV and Excel exports.
- **Local web UI** (`web.rs`, `assets/`) — loopback-only Axum server to search
  schools, run pulls, watch progress, preview results, and download all reports.
- **CLI** (`main.rs`) — `serve`, `pull`, `export-directory`, `import-schools`,
  and `validate-schools` subcommands.
- **Load harness** (`bin/stress.rs`) — in-process mock stress test that never
  contacts the public API; asserts completion, order preservation, and that the
  concurrency ceiling is never exceeded.
- **Documentation** — `README.md`, `docs/COMPATIBILITY.md` (behavior preserved
  and defects corrected versus the C++ project), and
  `docs/STATISTICAL_METHODS.md` (the reporting policy).

### Correctness fixes over the C++ implementation

- Missing, null, or wrong-typed numeric fields use nullable types instead of a
  numeric-zero default; a valid zero is preserved and never confused with
  missing data.
- A school selected in the UI is sent by CDS code and never fuzzy-matched again,
  eliminating duplicate-name mix-ups (e.g. the many "Lincoln Elementary").
- Every request has an explicit success / no-data / parse-error / fetch-error
  outcome; failures never masquerade as empty successful cards.
- Results are sorted deterministically after concurrent fetching, and retries no
  longer erase the indicator definition map.

### Statistical reporting

- Private measures and measures with a known denominator of 10 or fewer are
  fully suppressed; denominators of 11–29 carry a small-denominator caution.
- Only a nonzero official performance level is displayed; official `change` is
  never recomputed; favorable-direction gaps flip sign only for the two
  lower-is-better indicators.
- Year-specific cautions applied for 2020/2021 (informational), 2022
  (status-only), the 2023 College/Career Indicator, and 2025 Science.
- No fabricated inference (no confidence intervals, p-values, effect sizes,
  rankings, or composite scores) — the SummaryCards do not supply the inputs
  those methods require.

### UI

- Source attribution links to the official California School Dashboard
  (`https://www.caschooldashboard.org/`) with an inline, self-contained logo.
- A research-use disclaimer is shown in the UI and on every HTML and PDF export.
- Static assets are served with `no-cache` so rebuilt UI changes appear without
  a manual hard refresh.

### Testing

- Unit and integration tests cover parsing, matching, the client's retry and
  concurrency behavior, the statistical suppression rules, and every export
  format. Verified end-to-end against the live Dashboard API.
