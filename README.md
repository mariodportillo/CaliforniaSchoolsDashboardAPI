# California Dashboard API

A C++ client for fetching and parsing school performance data from the [California School Dashboard](https://www.caschooldashboard.org/) public API.

> **Rust rewrite available.** A Rust port with a local web UI and statistically
> grounded report generation (CSV, Excel, HTML, PDF) lives in
> [`california-dashboard-rs/`](california-dashboard-rs/). It preserves this
> project's documented behavior while correcting defects (see its
> [README](california-dashboard-rs/README.md) and
> [CHANGELOG](california-dashboard-rs/CHANGELOG.md)). Run `cargo run --release --
> serve` in that directory to open the UI.

## What It Does

The California School Dashboard publishes indicator data for every public school in the state — things like chronic absenteeism, suspension rates, graduation rates, ELA and math performance, and more. This project provides a programmatic way to pull that data by school name and year, without having to manually navigate the dashboard.

Given a set of years, the library:

1. Loads every active CA public school from a local SQLite database (`pubschls.db`)
2. Resolves school names to CDS (County-District-School) codes using a three-tier fuzzy matching strategy
3. Constructs the correct API endpoint URLs for each (school, year) pair
4. Fetches JSON responses concurrently using a pthreads worker pool and libcurl
5. Parses each response into structured `SummaryCard` objects
6. Stamps each card with its school name and year for downstream use

## Architecture

```
SchoolResolver          →   CaliforniaDashboardAPI   →   SummaryCard
(SQLite lookup,             (HTTP thread pool,            (data model)
 URL building,               rate limiting,
 fuzzy matching,             JSON parsing)
 metadata enrichment)
```

### SchoolResolver

Owns all school-identity logic. Opens `pubschls.db` at construction and pre-loads the CDS lookup into memory so all subsequent operations are pure in-memory with no repeated DB hits.

### CaliforniaDashboardAPI

Owns all network logic. Maintains a persistent libcurl worker pool with a token-bucket rate limiter. Default configuration: 50 workers, 1000 req/sec ceiling, 10s timeout.

### SummaryCard

Structured container for a single school/year response. Holds a vector of `Indicator` objects, each with its own CDS code, year ID, category, and performance data.

## Indicators Tracked

| ID | Category |
|----|----------|
| 1  | Chronic Absenteeism |
| 2  | Suspension Rate |
| 3  | English Learner Progress |
| 4  | Graduation Rate |
| 5  | College & Career Indicator |
| 6  | ELA Points Above/Below |
| 7  | Mathematics |
| 8  | Science |

## School Name Matching

School names are matched against the database using a three-tier strategy so you don't need to know the exact name as it appears in the state database:

1. **Exact match** — case-insensitive
2. **Substring match** — prefers the longest overlapping name to avoid false positives; requires a minimum overlap of 5 characters
3. **Fuzzy match** — Levenshtein edit distance, threshold of 5

Unmatched or inactive schools are skipped with a warning to stderr.

## Supported Years

| Year | Dashboard ID |
|------|-------------|
| 2017 | 3  |
| 2018 | 4  |
| 2019 | 5  |
| 2020 | 6  |
| 2021 | 7  |
| 2022 | 8  |
| 2023 | 9  |
| 2024 | 10 |
| 2025 | 11 |

## Dependencies

- [libcurl](https://curl.se/libcurl/) — HTTP requests
- [nlohmann/json](https://github.com/nlohmann/json) — JSON parsing
- [SQLite3](https://www.sqlite.org/) — local school database
- pthreads — concurrent fetching and metadata enrichment
- C++20 or later

## Setup

### 1. Download the school data

Download `pubschls.csv` from the [California Department of Education](https://www.cde.ca.gov/ds/si/ds/pubschls.asp) and place it in the `app/` directory.

### 2. Build the SQLite database

```bash
cd app
python3 csv_to_sqlite.py
```

This creates `pubschls.db` in the same directory. Only needs to be run once, or whenever you download a fresh CSV.

### 3. Validate the database (optional)

```bash
python3 validate_db.py           # full diff with per-row details
python3 validate_db.py --summary # summary only
```

Reports rows missing from the DB, phantom rows not in the CSV, and any field-level differences between the two.

### 4. Build the C++ project

```bash
cd app
mkdir build && cd build
cmake ..
make
```

## Usage

```cpp
#include "SchoolResolver.hh"
#include "CaliforniaDashboardAPI.hh"

int main() {
    // Opens pubschls.db and pre-loads CDS lookup into memory
    SchoolResolver resolver("../pubschls.db");

    // Builds URLs for every active school across the given years
    SchoolResolver::BuildResult result = resolver.buildURLs({"2022", "2023", "2024"});

    // Fetch all URLs concurrently
    CaliforniaDashboardAPI api;
    api.loadInURLs(result.urls);
    api.runFullURLFetch();

    // Stamp each card with its school name and year
    enrichCardsWithMetadata(api.allSummaryCardsVector, result.metadata);

    for (const auto& card : api.allSummaryCardsVector) {
        card.printIndicatorVector();
    }
}
```

To target specific schools instead of fetching everything:

```cpp
SchoolResolver::SchoolsMap schools = {
    {"Pomona High School",        {"2022", "2023"}},
    {"Diamond Ranch High School", {"2022", "2023"}},
};

SchoolResolver::BuildResult result = resolver.buildURLs(schools);
```

## Project Structure

```
app/
├── main.cpp                  # Entry point
├── SchoolResolver.hh/.cpp    # School lookup, URL building, metadata enrichment
├── CaliforniaDashboardAPI.hh/.cpp  # HTTP fetch pool
├── summaryCard.hh/.cpp       # SummaryCard and Indicator data model
├── pubschls.csv              # Source data (download from CDE)
├── pubschls.db               # SQLite cache of pubschls.csv
├── csv_to_sqlite.py          # Imports CSV into SQLite
├── validate_db.py            # Validates DB against CSV
└── CMakeLists.txt
```

## Data Source

School data is sourced from the California Department of Education's public schools list and the California School Dashboard API. This project is not affiliated with or endorsed by the California Department of Education or the California State Board of Education.

## License

MIT
