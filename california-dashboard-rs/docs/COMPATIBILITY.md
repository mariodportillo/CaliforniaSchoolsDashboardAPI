# Compatibility contract

The Rust application implements the documented behavior of the original C++
project while correcting defects that could return the wrong school or turn
missing data into a numeric zero.

## Preserved behavior

- Reads active schools with non-null `CDSCode` and `School` fields from the
  existing `schools` SQLite table.
- Preserves CDS codes as 14-character text, including leading zeroes.
- Supports Dashboard years 2017 through 2025 with IDs 3 through 11.
- Builds URLs as
  `https://api.caschooldashboard.org/Reports/{CDS}/{year-id}/SummaryCards`.
- Offers case-insensitive exact, substring, and Levenshtein matching, with a
  maximum fuzzy distance of five.
- Parses a SummaryCards array or a single indicator object.
- Retains the exact raw JSON, exact `primary`/`secondary` blocks, category map,
  metadata, printing, and JSON save/load operations from the C++ `SummaryCard`.
- Skips malformed/non-object indicator entries individually instead of losing
  the rest of a valid card.
- Recognizes indicator IDs 1 through 8 and retains unknown indicators.
- Uses 50 concurrent requests, a 1,000 request/second ceiling, and a 10-second
  per-attempt timeout by default.
- Uses four total attempts with exponential retry delays for transient
  transport failures and additionally handles HTTP 408, 429, and 5xx responses.

## Intentional correctness fixes

- A school selected in the UI is sent by CDS code and is never fuzzy-matched a
  second time. This fixes duplicate names such as the many schools named
  "Lincoln Elementary."
- Missing, null, or wrong-typed numeric fields use nullable Rust types rather
  than the C++ parser's zero default.
- Both primary and secondary measures are typed.
- Request metadata is attached before the fetch, so an empty response cannot
  lose its school/year identity.
- Each request has an explicit success, no-data, parse-error, or fetch-error
  outcome. Failed requests do not masquerade as empty successful cards.
- Results are sorted deterministically after concurrent fetching.
- Retries do not erase the indicator definition map.
- The rate and concurrency limits also cover retries.
- UI jobs share a single scheduler so multiple browser requests cannot multiply
  the configured global concurrency or request-rate ceiling.

## Added UI-only surfaces

- The browser can choose explicit schools or run the original all-active-school
  workflow for any supported year set.
- Directory CSV/Excel downloads use the supplied CDE workbook's five-column
  schema and include an actual filterable Excel table and total formula.
- Report CSV, Excel, HTML, and PDF files all consume the same canonical rows.

## Data files

The Rust project does not duplicate the supplied school database. By default
the command searches for `pubschls.db` in the current directory and its parent,
or accepts an explicit path. The original database remains read-only.
