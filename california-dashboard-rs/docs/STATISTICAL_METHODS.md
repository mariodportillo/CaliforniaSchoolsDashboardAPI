# Statistical reporting policy

Version: `2026-07-descriptive-v2`

This application reports California School Dashboard results as descriptive,
administrative aggregates. It does not treat them as a random sample and does
not manufacture inferential statistics that the source data cannot support.

## Canonical disclosure policy

The same disclosure rules are applied before data reach the browser, CSV,
Excel, HTML, or PDF output.

- A missing value remains missing. A valid zero remains zero.
- A measure marked `isPrivateData` never exposes its status, change,
  performance level, group colors, comparator difference, or derived trend.
- When a known count is 10 or fewer, numeric results are defensively
  suppressed even if the payload contains them.
- Counts from 11 through 29 receive a small-denominator warning and are shown
  only as descriptive results. The app never synthesizes an accountability
  color.
- Only an official performance value supplied by the API is displayed; zero is
  retained as a valid published value.
- Suppressed and missing observations are excluded from calculations rather
  than imputed.

## Units and direction

| ID | Indicator | Unit | Favorable direction |
|---:|---|---|---|
| 1 | Chronic Absenteeism | percentage points | lower |
| 2 | Suspension Rate | percentage points | lower |
| 3 | English Learner Progress | percentage points | higher |
| 4 | Graduation Rate | percentage points | higher |
| 5 | College/Career Indicator | percentage points | higher |
| 6 | ELA | distance-from-standard points | higher |
| 7 | Mathematics | distance-from-standard points | higher |
| 8 | Science | Science Points (0–100) | higher |

The report never averages different indicators or units. Negative ELA and
mathematics values are valid, so the app uses absolute point differences and
never presents relative percentage changes for those indicators.

## Comparisons and changes

- `change` is the official API value and is never overwritten by a recomputed
  trend.
- A primary-to-secondary gap is an absolute difference in the indicator's
  natural unit. The report calls it a *CDE comparison* because the API contract
  available to this project does not document the secondary payload as an
  independent statewide sample.
- The favorable-direction gap reverses the raw sign only for the two
  lower-is-better indicators. It is labeled a descriptive difference, not an
  effect size or significance test.
- Colors and the red/orange/yellow/green/blue fields are official categories
  and counts of reportable groups. They are never converted into student
  percentages or a composite score.

## Time-series cautions

- Missing years break a trend; the app does not interpolate or annualize over
  a gap.
- The 2020 and 2021 releases are labeled informational because production of
  state indicators was suspended during the pandemic.
- 2022 results are treated as Status-only where Change/colors were unavailable.
- The 2023 College/Career Indicator is not assigned a synthesized color.
- 2025 Science publishes status, change, and colors, but is labeled
  informational and is not used for accountability; those published fields are
  retained rather than erased.
- A line across years describes different annual populations and is not a
  longitudinal student-cohort analysis.

## Deliberately excluded analyses

The available SummaryCards do not provide raw numerators, outcome variances,
cohort linkage, covariance, or an independent comparison sample. Therefore the
default report does not produce standard errors, confidence intervals,
p-values, Cohen's d, significance stars, causal claims, school rankings, or a
single composite score. The secondary aggregate may include the selected
school, which also invalidates an independent two-sample test.

## Provenance and limitations

Every report identifies the selected school by CDS code, dashboard year,
source URL, retrieval status, software and method version, and completeness
counts. It also states that:

- the data are administrative and observational;
- school populations, grade spans, and students can change between years;
- missing or suppressed values may be nonrandom; and
- school-level aggregates can mask subgroup differences.

## Official references

- California Department of Education Dashboard Technical Guide:
  https://www.cde.ca.gov/ta/ac/cm/dashboardguide.asp
- Data displayed on the 2025 Dashboard:
  https://www.cde.ca.gov/ta/ac/cm/documents/whatdatawillbeused25.pdf
- Dashboard resources and downloadable indicator files:
  https://www.cde.ca.gov/ta/ac/cm/dashboardresources.asp
- Dashboard release timeline:
  https://www.cde.ca.gov/ta/ac/cm/documents/dashboardreleasetime25.pdf
