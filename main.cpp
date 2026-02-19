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
        std::cout << "[INFO] Exact match: \"" << schoolName
                  << "\" -> \"" << originalNames.at(query) << "\"\n";
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
        std::cout << "[INFO] Substring match: \"" << schoolName
                  << "\" -> \"" << originalNames.at(substrMatchKey) << "\"\n";
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
        std::cout << "[INFO] Fuzzy match (edit distance=" << bestDist << "): \""
                  << schoolName << "\" -> \""
                  << originalNames.at(bestKey) << "\"\n";
        return cdsLookup.at(bestKey);
    }

    std::cerr << "[WARN] No match found for school: \"" << schoolName << "\"";
    if (!bestKey.empty())
        std::cerr << " (closest: \"" << originalNames.at(bestKey)
                  << "\", edit distance=" << bestDist << ")";
    std::cerr << "\n";
    return "";
}

// =============================================================================
// buildURLVectorForSchools
// =============================================================================

/**
 * Populates `urls` with CA School Dashboard API endpoints for every valid
 * (school, year) pair found in `schools`.
 *
 * @param urls     Output vector that receives fully-formed URL strings.
 * @param schools  Map of { schoolName -> list of year strings (e.g. "2023") }.
 *
 * Matching is case-insensitive and uses a three-tier strategy
 * (exact -> substring -> fuzzy) to find the best school name in the CSV.
 * Schools with no suitable match and unrecognised year strings are skipped
 * with a warning to stderr.
 *
 * URL format: BASE_URL + CDSCode + "/" + yearId + "/SummaryCards"
 */
void buildURLVectorForSchools(std::vector<std::string>& urls,
                               std::map<std::string, std::vector<std::string>>& schools) {
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
        if (cds.empty()) continue; // no match within threshold, warning already logged

        for (const std::string& year : years) {
            if (!validateYear(year)) continue;
            const std::string& yearId = YEAR_TO_ID.at(year);
            urls.push_back(BASE_URL + cds + "/" + yearId + "/SummaryCards");
        }
    }
}

// =============================================================================
// main
// =============================================================================

int main()
{
    CaliforniaDashboardAPI api;
    std::vector<std::string> urls;
    std::vector<std::string> years = {"2022", "2023", "2024"};
    std::map<std::string, std::vector<std::string>> schools = {
        {"Pomona High School",        years},
        {"Diamond Ranch High School", years},
        {"Garey High School", years},
        {"Palo Alto High School", years}
    };

    buildURLVectorForSchools(urls, schools);
    
    if (!api.loadInURLs(urls)) {
        std::cerr << "Failed to load URLs" << std::endl;
        return 1;
    }

    std::cout << "Fetching data from API..." << std::endl;

    if (!api.runFullURLFetch()) {
        std::cerr << "Failed to fetch data" << std::endl;
        return 1;
    }

    std::cout << "\nData fetched successfully!" << std::endl;

    for (const auto& card : api.allSummaryCardsVector) {
        std::cout << "\n=== Card ===" << std::endl;
        card.printIndicatorVector();
    }

    return 0;
}
