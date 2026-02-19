#include "summaryCard.hh"
#include "CaliforniaDashboardAPI.hh"
#include <iostream>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>
#include <map>
#include <unordered_map>
#include <algorithm>
#include <stdexcept>
#include <thread>
#include <pthread.h>

// =============================================================================
// Constants
// =============================================================================

static const std::string BASE_URL = "https://api.caschooldashboard.org/Reports/";

static const std::map<std::string, std::string> YEAR_TO_ID = {
    {"2017", "3"}, {"2018", "4"}, {"2019", "5"}, {"2020", "6"},
    {"2021", "7"}, {"2022", "8"}, {"2023", "9"}, {"2024", "10"}, {"2025", "11"}
};

// =============================================================================
// String Utilities
// =============================================================================

// Converts a string to lowercase.
static std::string toLower(const std::string& s) {
    std::string result = s;
    std::transform(result.begin(), result.end(), result.begin(), ::tolower);
    return result;
}

// Trims leading/trailing whitespace and surrounding quotes from a CSV field.
static std::string trimField(const std::string& s) {
    size_t start = s.find_first_not_of(" \t\r\n\"");
    size_t end   = s.find_last_not_of(" \t\r\n\"");
    return (start == std::string::npos) ? "" : s.substr(start, end - start + 1);
}

// =============================================================================
// CSV Parsing
// =============================================================================

// Parses one CSV line respecting quoted fields (commas inside quotes are ignored).
static std::vector<std::string> parseCSVLine(const std::string& line) {
    std::vector<std::string> fields;
    std::string field;
    bool inQuotes = false;

    for (size_t i = 0; i < line.size(); ++i) {
        char c = line[i];
        if (c == '"') {
            // Handle escaped quotes ("") inside a quoted field.
            if (inQuotes && i + 1 < line.size() && line[i + 1] == '"') {
                field += '"';
                ++i;
            } else {
                inQuotes = !inQuotes;
            }
        } else if (c == ',' && !inQuotes) {
            fields.push_back(trimField(field));
            field.clear();
        } else {
            field += c;
        }
    }
    fields.push_back(trimField(field)); // last field
    return fields;
}

// Builds two maps from the CSV:
//   cdsLookup     : { lowercaseSchoolName -> CDSCode }
//   originalNames : { lowercaseSchoolName -> originalSchoolName }
// Only "Active" schools are included.
// Column indices (0-based): 0 = CDSCode, 3 = StatusType, 6 = School
static std::unordered_map<std::string, std::string> buildCDSLookup(
    const std::string& csvPath,
    std::unordered_map<std::string, std::string>& originalNames)
{
    std::unordered_map<std::string, std::string> lookup;

    std::ifstream file(csvPath);
    if (!file.is_open())
        throw std::runtime_error("Cannot open CSV file: " + csvPath);

    std::string line;
    bool isHeader = true;

    while (std::getline(file, line)) {
        // Strip UTF-8 BOM on the very first line.
        if (isHeader) {
            if (line.size() >= 3 &&
                static_cast<unsigned char>(line[0]) == 0xEF &&
                static_cast<unsigned char>(line[1]) == 0xBB &&
                static_cast<unsigned char>(line[2]) == 0xBF)
                line.erase(0, 3);
            isHeader = false;
            continue; // skip header row
        }

        auto fields = parseCSVLine(line);
        if (fields.size() < 7) continue;

        const std::string& cds        = fields[0]; // CDSCode
        const std::string& statusType = fields[3]; // StatusType
        const std::string& school     = fields[6]; // School

        if (school.empty() || cds.empty()) continue;
        if (statusType != "Active") continue; // skip closed/pending schools

        std::string key     = toLower(school);
        lookup[key]         = cds;
        originalNames[key]  = school;
    }

    return lookup;
}

// =============================================================================
// Validation
// =============================================================================

// Returns true if the year string is a recognised dashboard year.
static bool validateYear(const std::string& year) {
    if (YEAR_TO_ID.find(year) == YEAR_TO_ID.end()) {
        std::cerr << "[WARN] Year not supported by dashboard API: \""
                  << year << "\"\n";
        return false;
    }
    return true;
}

// =============================================================================
// Fuzzy Matching
// =============================================================================

// Computes the Levenshtein edit distance between two strings.
static size_t editDistance(const std::string& a, const std::string& b) {
    size_t m = a.size(), n = b.size();
    std::vector<std::vector<size_t>> dp(m + 1, std::vector<size_t>(n + 1));
    for (size_t i = 0; i <= m; ++i) dp[i][0] = i;
    for (size_t j = 0; j <= n; ++j) dp[0][j] = j;
    for (size_t i = 1; i <= m; ++i)
        for (size_t j = 1; j <= n; ++j)
            dp[i][j] = (a[i-1] == b[j-1])
                ? dp[i-1][j-1]
                : 1 + std::min({dp[i-1][j], dp[i][j-1], dp[i-1][j-1]});
    return dp[m][n];
}

// Maximum edit distance allowed for a fuzzy match to be accepted.
static const size_t MAX_EDIT_DISTANCE = 5;

// Finds the best matching CDS code for a given school name using a three-tier strategy:
//   1. Exact match        (case-insensitive)
//   2. Substring match    (query contained in a school name, or vice versa)
//   3. Closest Levenshtein distance (within MAX_EDIT_DISTANCE)
// Returns the CDSCode string, or an empty string if no suitable match is found.
static std::string findBestMatch(
    const std::string& schoolName,
    const std::unordered_map<std::string, std::string>& cdsLookup,
    const std::unordered_map<std::string, std::string>& originalNames)
{
    std::string query = toLower(schoolName);

    // -- Tier 1: Exact match (case-insensitive) --
    auto it = cdsLookup.find(query);
    if (it != cdsLookup.end()) {
        return it->second;
    }

    // -- Tier 2: Substring match --
    // Prefer the LONGEST candidate that overlaps, giving the most specific match.
    // A minimum overlap of 5 chars guards against short noise tokens like "Pomo".
    static const size_t MIN_SUBSTR_LEN = 5;

    std::string substrMatchKey;
    size_t substrMatchLen = 0; // tracking longest, not shortest

    for (const auto& [key, cds] : cdsLookup) {
        bool overlap = (key.find(query) != std::string::npos ||
                        query.find(key) != std::string::npos);
        if (overlap && key.size() >= MIN_SUBSTR_LEN && key.size() > substrMatchLen) {
            substrMatchLen = key.size();
            substrMatchKey = key;
        }
    }

    if (!substrMatchKey.empty()) {
        return cdsLookup.at(substrMatchKey);
    }

    // -- Tier 3: Levenshtein fuzzy match --
    std::string bestKey;
    size_t bestDist = std::string::npos;

    for (const auto& [key, cds] : cdsLookup) {
        size_t dist = editDistance(query, key);
        if (dist < bestDist) {
            bestDist = dist;
            bestKey  = key;
        }
    }

    if (!bestKey.empty() && bestDist <= MAX_EDIT_DISTANCE) {
        return cdsLookup.at(bestKey);
    }

    return "";
}

// =============================================================================
// buildURLVectorForSchools
// =============================================================================

/**
 * Populates `urls` with CA School Dashboard API endpoints for every valid
 * (school, year) pair found in `schools`. Also populates `urlMetadata` which
 * maps each URL to its (schoolName, year) for post-fetch card enrichment.
 *
 * @param urls        Output vector of fully-formed URL strings.
 * @param schools     Map of { schoolName -> list of year strings (e.g. "2023") }.
 * @param urlMetadata Output map of { url -> (schoolName, year) }.
 *
 * Matching is case-insensitive with a three-tier strategy:
 * exact -> substring -> fuzzy (Levenshtein, threshold MAX_EDIT_DISTANCE).
 * Unmatched schools and unsupported years are skipped with a warning.
 *
 * URL format: BASE_URL + CDSCode + "/" + yearId + "/SummaryCards"
 */
void buildURLVectorForSchools(
    std::vector<std::string>& urls,
    std::map<std::string, std::vector<std::string>>& schools,
    std::map<std::string, std::pair<std::string, std::string>>& urlMetadata)
{
    static const std::string CSV_PATH = "../pubschls.csv";

    std::unordered_map<std::string, std::string> cdsLookup;
    std::unordered_map<std::string, std::string> originalNames;

    try {
        cdsLookup = buildCDSLookup(CSV_PATH, originalNames);
    } catch (const std::exception& e) {
        std::cerr << "[ERROR] Failed to load CDS lookup: " << e.what() << "\n";
        return;
    }

    for (const auto& [schoolName, years] : schools) {
        std::string cds = findBestMatch(schoolName, cdsLookup, originalNames);
        if (cds.empty()) continue;

        for (const std::string& year : years) {
            if (!validateYear(year)) continue;
            const std::string& yearId = YEAR_TO_ID.at(year);
            std::string url = BASE_URL + cds + "/" + yearId + "/SummaryCards";
            urls.push_back(url);
            // Record which school + year this URL belongs to so we can stamp
            // the card after fetching (API responses only contain CDS codes).
            urlMetadata[url] = {schoolName, year};
        }
    }
}

// =============================================================================
// enrichCardsWithMetadata
// =============================================================================

// Arg passed to each stamping thread — owns a slice of the cards vector.
struct EnrichArg {
    std::vector<SummaryCard>* cards;
    size_t start;
    size_t end; // exclusive
    const std::unordered_map<std::string, std::pair<std::string, std::string>>* lookup;
    // lookup key: cdsCode + ":" + yearId  ->  (schoolName, year)
};

static void* enrichWorker(void* raw) {
    auto* a = static_cast<EnrichArg*>(raw);
    for (size_t i = a->start; i < a->end; ++i) {
        SummaryCard& card = (*a->cards)[i];
        const auto& indicators = card.getIndicatorVector();
        if (indicators.empty()) continue;

        // All indicators in a card share the same cdsCode and schoolYearId.
        const std::string key = indicators[0].cdsCode + ":"
                              + std::to_string(indicators[0].schoolYearId);

        auto it = a->lookup->find(key);
        if (it != a->lookup->end())
            card.setMetadata(it->second.first, it->second.second);
    }
    return nullptr;
}

/**
 * Builds a flat lookup from (cdsCode:yearId) -> (schoolName, year) by parsing
 * each URL in urlMetadata, then spawns one thread per hardware core to stamp
 * every card in allSummaryCardsVector with its school name and year.
 *
 * No locks needed: each thread owns a disjoint slice of the cards vector and
 * the lookup map is read-only after construction.
 *
 * @param cards       The vector of fetched SummaryCards to enrich (mutated).
 * @param urlMetadata URL -> (schoolName, year) built during URL construction.
 */
void enrichCardsWithMetadata(
    std::vector<SummaryCard>& cards,
    const std::map<std::string, std::pair<std::string, std::string>>& urlMetadata)
{
    if (cards.empty() || urlMetadata.empty()) return;

    // --- Build the flat lookup (single-threaded, O(n), negligible cost) ---
    // Key: "cdsCode:yearId"  e.g. "19649071937028:7"
    std::unordered_map<std::string, std::pair<std::string, std::string>> lookup;
    lookup.reserve(urlMetadata.size());

    for (const auto& [url, meta] : urlMetadata) {
        // URL format: BASE_URL + cdsCode + "/" + yearId + "/SummaryCards"
        std::string stripped = url.substr(BASE_URL.size());
        size_t slash1 = stripped.find('/');
        size_t slash2 = stripped.find('/', slash1 + 1);
        if (slash1 == std::string::npos || slash2 == std::string::npos) continue;
        std::string cds    = stripped.substr(0, slash1);
        std::string yearId = stripped.substr(slash1 + 1, slash2 - slash1 - 1);
        lookup[cds + ":" + yearId] = meta;
    }

    // --- Spawn one thread per logical core, each owning a slice of cards ---
    const size_t n        = cards.size();
    const size_t nThreads = std::max(1u, std::thread::hardware_concurrency());
    const size_t chunk    = (n + nThreads - 1) / nThreads; // ceiling division

    std::vector<EnrichArg>    args(nThreads);
    std::vector<pthread_t>    tids(nThreads);
    size_t                    spawned = 0;

    for (size_t t = 0; t < nThreads; ++t) {
        size_t start = t * chunk;
        if (start >= n) break; // fewer cards than cores

        args[t] = { &cards, start, std::min(start + chunk, n), &lookup };

        int err = pthread_create(&tids[t], nullptr, enrichWorker, &args[t]);
        if (err) {
            fprintf(stderr, "[WARN] enrichCardsWithMetadata: pthread_create failed: %s\n",
                    strerror(err));
            // Fall back: do this slice on the calling thread
            enrichWorker(&args[t]);
        } else {
            ++spawned;
        }
    }

    for (size_t t = 0; t < spawned; ++t)
        pthread_join(tids[t], nullptr);
}


// =============================================================================
// buildAllSchoolsMap
// =============================================================================

/**
 * Reads every active school from pubschls.csv and returns a map of
 * { originalSchoolName -> years } ready to pass into buildURLVectorForSchools.
 *
 * Duplicate school names (same name, different districts) are deduplicated by
 * appending the CDSCode in parentheses so both entries are preserved:
 *   "Lincoln High (19647331934609)"
 *   "Lincoln High (19730106053658)"
 *
 * @param years   The year strings to assign to every school (e.g. {"2022","2023"}).
 * @param csvPath Path to pubschls.csv (default: "../pubschls.csv").
 * @returns       Map of { schoolName -> years }.
 */
std::map<std::string, std::vector<std::string>> buildAllSchoolsMap(
    const std::vector<std::string>& years,
    const std::string& csvPath = "../pubschls.csv")
{
    std::map<std::string, std::vector<std::string>> schools;

    std::ifstream file(csvPath);
    if (!file.is_open()) {
        std::cerr << "[ERROR] buildAllSchoolsMap: cannot open CSV: " << csvPath << "\n";
        return schools;
    }

    // Track seen names so duplicates get a CDS suffix.
    std::unordered_map<std::string, int> nameSeen;

    std::string line;
    bool isHeader = true;

    while (std::getline(file, line)) {
        // Strip UTF-8 BOM on first line.
        if (isHeader) {
            if (line.size() >= 3 &&
                static_cast<unsigned char>(line[0]) == 0xEF &&
                static_cast<unsigned char>(line[1]) == 0xBB &&
                static_cast<unsigned char>(line[2]) == 0xBF)
                line.erase(0, 3);
            isHeader = false;
            continue;
        }

        auto fields = parseCSVLine(line);
        if (fields.size() < 7) continue;

        const std::string& cds        = fields[0]; // CDSCode
        const std::string& statusType = fields[3]; // StatusType
        const std::string& school     = fields[6]; // School

        if (school.empty() || cds.empty()) continue;
        if (statusType != "Active") continue;

        std::string key = school;

        // On first collision rename the original entry with its CDS suffix,
        // then every subsequent collision also gets one.
        if (nameSeen[school]++ == 1) {
            // The original (collision-free) entry needs renaming too.
            // Find it and re-insert under a disambiguated key.
            auto it = schools.find(school);
            if (it != schools.end()) {
                // We don't have the original CDS here, so mark it as ambiguous.
                std::string disambig = school + " (ambiguous)";
                schools[disambig] = std::move(it->second);
                schools.erase(it);
            }
        }

        if (nameSeen[school] > 1)
            key = school + " (" + cds + ")";

        schools[key] = years;
    }

    std::cout << "[INFO] buildAllSchoolsMap: loaded " << schools.size()
              << " active schools from CSV.\n";
    return schools;
}

// =============================================================================
// main
// =============================================================================

int main()
{
    CaliforniaDashboardAPI api;
    std::vector<std::string> urls;
    std::vector<std::string> years = {"2021", "2022", "2023", "2024"};

    // Build a schools map containing every active CA public school.
    // Swap this for a hand-crafted map to target specific schools instead.
    std::map<std::string, std::vector<std::string>> schools =
        buildAllSchoolsMap(years);

    // urlMetadata maps each URL -> (schoolName, year) so cards can be labelled
    // after fetching, since the API responses only contain CDS codes.
    std::map<std::string, std::pair<std::string, std::string>> urlMetadata;

    buildURLVectorForSchools(urls, schools, urlMetadata);

    if (!api.loadInURLs(urls)) {
        std::cerr << "Failed to load URLs" << std::endl;
        return 1;
    }

    std::cout << "Fetching data from API..." << std::endl;

    if (!api.runFullURLFetch()) {
        std::cerr << "Failed to fetch data" << std::endl;
        return 1;
    }

    enrichCardsWithMetadata(api.allSummaryCardsVector, urlMetadata);

    std::cout << "\nData fetched successfully!" << std::endl;

    for (const auto& card : api.allSummaryCardsVector) {
        std::cout << "\n=== Card ===" << std::endl;
        card.printIndicatorVector();
    }

    return 0;
}
