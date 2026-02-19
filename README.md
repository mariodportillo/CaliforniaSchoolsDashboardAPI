# California Dashboard API

A C++ client for fetching and parsing school performance data from the [California School Dashboard](https://www.caschooldashboard.org/) public API.

## What It Does

The California School Dashboard publishes indicator data for every public school in the state — things like chronic absenteeism, suspension rates, graduation rates, ELA and math performance, and more. This project provides a programmatic way to pull that data by school name and year, without having to manually navigate the dashboard.

Given a map of school names and years, the library:

1. Looks up each school's CDS (County-District-School) code from the California public schools dataset (`pubschls.csv`)
2. Validates the school is currently active
3. Constructs the correct API endpoint URLs
4. Fetches the JSON responses concurrently using pthreads and libcurl
5. Parses each response into structured `SummaryCard` objects for use in your program

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

School names are matched against the CSV using a three-tier strategy so you don't need to know the exact name as it appears in the state database:

1. **Exact match** — case-insensitive
2. **Substring match** — prefers the longest overlapping name to avoid false positives
3. **Fuzzy match** — Levenshtein edit distance, with a configurable threshold

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
- pthreads — concurrent fetching
- C++17 or later

## Building

```bash
mkdir build && cd build
cmake ..
make
```

The CSV file `pubschls.csv` must be in the parent directory of the build folder (`../pubschls.csv`). The latest version of the file can be downloaded from the [California Department of Education](https://www.cde.ca.gov/ds/si/ds/pubschls.asp).

## Usage

```cpp
CaliforniaDashboardAPI api;
std::vector<std::string> urls;
std::vector<std::string> years = {"2022", "2023", "2024"};

std::map<std::string, std::vector<std::string>> schools = {
    {"Pomona High School",        years},
    {"Diamond Ranch High School", years}
};

buildURLVectorForSchools(urls, schools);
api.loadInURLs(urls);
api.runFullURLFetch();

for (const auto& card : api.allSummaryCardsVector) {
    card.printIndicatorVector();
}
```

## Data Source

School data is sourced from the California Department of Education's public schools list and the California School Dashboard API. This project is not affiliated with or endorsed by the California Department of Education or the California State Board of Education.

## License

MIT
