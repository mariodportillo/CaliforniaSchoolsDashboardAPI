#include "SchoolResolver.hh"
#include <algorithm>
#include <iostream>
#include <stdexcept>
#include <vector>
#include <thread>
#include <pthread.h>
#include <cstring>
// =============================================================================
// Static data
// =============================================================================

const std::map<std::string, std::string> SchoolResolver::YEAR_TO_ID = {
    {"2017", "3"}, {"2018", "4"}, {"2019", "5"}, {"2020", "6"},
    {"2021", "7"}, {"2022", "8"}, {"2023", "9"}, {"2024", "10"}, {"2025", "11"}
};

// =============================================================================
// Construction / destruction
// =============================================================================

SchoolResolver::SchoolResolver(const std::string& dbPath)
    : dbPath_(dbPath)
{
    int rc = sqlite3_open_v2(dbPath_.c_str(), &db_, SQLITE_OPEN_READONLY, nullptr);
    if (rc != SQLITE_OK) {
        std::string err = db_ ? sqlite3_errmsg(db_) : "unknown error";
        sqlite3_close(db_);
        db_ = nullptr;
        throw std::runtime_error("SchoolResolver: cannot open DB '" + dbPath_ + "': " + err);
    }
    loadCDSLookup();
}

SchoolResolver::~SchoolResolver() {
    if (db_) sqlite3_close(db_);
}

// =============================================================================
// loadCDSLookup  —  SELECT once at construction, store in memory
// =============================================================================

void SchoolResolver::loadCDSLookup() {
    const char* sql =
        "SELECT CDSCode, School "
        "FROM schools "
        "WHERE StatusType = 'Active' "
        "  AND CDSCode IS NOT NULL "
        "  AND School  IS NOT NULL;";

    sqlite3_stmt* stmt = nullptr;
    int rc = sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
    if (rc != SQLITE_OK) {
        throw std::runtime_error(
            std::string("SchoolResolver: prepare failed: ") + sqlite3_errmsg(db_));
    }

    while (sqlite3_step(stmt) == SQLITE_ROW) {
        const char* rawCDS    = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 0));
        const char* rawSchool = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 1));
        if (!rawCDS || !rawSchool) continue;

        std::string cds    = rawCDS;
        std::string school = rawSchool;
        std::string key    = toLower(school);

        cdsLookup_[key]     = cds;
        originalNames_[key] = school;
    }

    sqlite3_finalize(stmt);
    std::cout << "[INFO] SchoolResolver: loaded " << cdsLookup_.size()
              << " active schools from '" << dbPath_ << "'\n";
}

// =============================================================================
// buildAllSchoolsMap
// =============================================================================

SchoolResolver::SchoolsMap
SchoolResolver::buildAllSchoolsMap(const std::vector<std::string>& years) const {
    SchoolsMap schools;

    const char* sql =
        "SELECT CDSCode, School "
        "FROM schools "
        "WHERE StatusType = 'Active' "
        "  AND CDSCode IS NOT NULL "
        "  AND School  IS NOT NULL;";

    sqlite3_stmt* stmt = nullptr;
    int rc = sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
    if (rc != SQLITE_OK) {
        std::cerr << "[ERROR] buildAllSchoolsMap: " << sqlite3_errmsg(db_) << "\n";
        return schools;
    }

    // First pass: count name collisions
    std::unordered_map<std::string, int> nameSeen;
    std::vector<std::pair<std::string, std::string>> rows; // (cds, school)

    while (sqlite3_step(stmt) == SQLITE_ROW) {
        const char* rawCDS    = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 0));
        const char* rawSchool = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 1));
        if (!rawCDS || !rawSchool) continue;
        rows.emplace_back(rawCDS, rawSchool);
        nameSeen[rawSchool]++;
    }
    sqlite3_finalize(stmt);

    // Second pass: build map, disambiguating duplicates with CDS suffix
    for (const auto& [cds, school] : rows) {
        std::string key = (nameSeen[school] > 1)
            ? school + " (" + cds + ")"
            : school;
        schools[key] = years;
    }

    std::cout << "[INFO] SchoolResolver::buildAllSchoolsMap: "
              << schools.size() << " entries built.\n";
    return schools;
}

// =============================================================================
// buildURLs  (from SchoolsMap)
// =============================================================================

SchoolResolver::BuildResult
SchoolResolver::buildURLs(const SchoolsMap& schools) const {
    BuildResult result;

    for (const auto& [schoolName, years] : schools) {
        std::string cds = findBestMatch(schoolName);
        if (cds.empty()) {
            std::cerr << "[WARN] SchoolResolver: no CDS match for \"" << schoolName << "\"\n";
            continue;
        }

        for (const std::string& year : years) {
            if (!validateYear(year)) continue;
            const std::string& yearId = YEAR_TO_ID.at(year);
            std::string url = std::string(BASE_URL) + cds + "/" + yearId + "/SummaryCards";
            result.urls.push_back(url);
            result.metadata[url] = {schoolName, year};
        }
    }

    return result;
}

// =============================================================================
// buildURLs  (convenience overload — builds all active schools internally)
// =============================================================================

SchoolResolver::BuildResult
SchoolResolver::buildURLs(const std::vector<std::string>& years) const {
    return buildURLs(buildAllSchoolsMap(years));
}

// =============================================================================
// findBestMatch  —  3-tier: exact -> substring -> Levenshtein
// =============================================================================

std::string SchoolResolver::findBestMatch(const std::string& schoolName) const {
    std::string query = toLower(schoolName);

    // Tier 1: exact (case-insensitive)
    auto it = cdsLookup_.find(query);
    if (it != cdsLookup_.end()) return it->second;

    // Tier 2: substring  (longest overlapping candidate wins)
    static constexpr size_t MIN_SUBSTR_LEN = 5;
    std::string substrKey;
    size_t      substrLen = 0;

    for (const auto& [key, cds] : cdsLookup_) {
        bool overlap = (key.find(query) != std::string::npos ||
                        query.find(key)  != std::string::npos);
        if (overlap && key.size() >= MIN_SUBSTR_LEN && key.size() > substrLen) {
            substrLen = key.size();
            substrKey = key;
        }
    }
    if (!substrKey.empty()) return cdsLookup_.at(substrKey);

    // Tier 3: Levenshtein fuzzy match
    std::string bestKey;
    size_t      bestDist = std::string::npos;

    for (const auto& [key, cds] : cdsLookup_) {
        size_t dist = editDistance(query, key);
        if (dist < bestDist) {
            bestDist = dist;
            bestKey  = key;
        }
    }

    if (!bestKey.empty() && bestDist <= MAX_EDIT_DISTANCE)
        return cdsLookup_.at(bestKey);

    return "";
}

struct EnrichArg {
    std::vector<SummaryCard>* cards;
    size_t start;
    size_t end;
    const std::unordered_map<std::string, std::pair<std::string, std::string>>* lookup;
};

static void* enrichWorker(void* raw) {
    auto* a = static_cast<EnrichArg*>(raw);
    for (size_t i = a->start; i < a->end; ++i) {
        SummaryCard& card = (*a->cards)[i];
        const auto& indicators = card.getIndicatorVector();
        if (indicators.empty()) continue;

        const std::string key = indicators[0].cdsCode + ":"
                              + std::to_string(indicators[0].schoolYearId);

        auto it = a->lookup->find(key);
        if (it != a->lookup->end())
            card.setMetadata(it->second.first, it->second.second);
    }
    return nullptr;
}

void enrichCardsWithMetadata(
    std::vector<SummaryCard>& cards,
    const SchoolResolver::URLMetadata& urlMetadata)
{
    if (cards.empty() || urlMetadata.empty()) return;

    // Build flat lookup: "cdsCode:yearId" -> (schoolName, year)
    std::unordered_map<std::string, std::pair<std::string, std::string>> lookup;
    lookup.reserve(urlMetadata.size());

    for (const auto& [url, meta] : urlMetadata) {
        // URL format: BASE_URL + cdsCode + "/" + yearId + "/SummaryCards"
        std::string stripped = url.substr(std::string("https://api.caschooldashboard.org/Reports/").size());
        size_t slash1 = stripped.find('/');
        size_t slash2 = stripped.find('/', slash1 + 1);
        if (slash1 == std::string::npos || slash2 == std::string::npos) continue;
        std::string cds    = stripped.substr(0, slash1);
        std::string yearId = stripped.substr(slash1 + 1, slash2 - slash1 - 1);
        lookup[cds + ":" + yearId] = meta;
    }

    // Spawn one thread per logical core, each owning a slice of cards
    const size_t n        = cards.size();
    const size_t nThreads = std::max(1u, std::thread::hardware_concurrency());
    const size_t chunk    = (n + nThreads - 1) / nThreads;

    std::vector<EnrichArg> args(nThreads);
    std::vector<pthread_t> tids(nThreads);
    size_t spawned = 0;

    for (size_t t = 0; t < nThreads; ++t) {
        size_t start = t * chunk;
        if (start >= n) break;

        args[t] = { &cards, start, std::min(start + chunk, n), &lookup };

        int err = pthread_create(&tids[t], nullptr, enrichWorker, &args[t]);
        if (err) {
            fprintf(stderr, "[WARN] enrichCardsWithMetadata: pthread_create failed: %s\n",
                    strerror(err));
            enrichWorker(&args[t]); // fallback: run on calling thread
        } else {
            ++spawned;
        }
    }

    for (size_t t = 0; t < spawned; ++t)
        pthread_join(tids[t], nullptr);
}

// =============================================================================
// Utilities
// =============================================================================

bool SchoolResolver::validateYear(const std::string& year) {
    if (YEAR_TO_ID.find(year) == YEAR_TO_ID.end()) {
        std::cerr << "[WARN] SchoolResolver: unsupported year \"" << year << "\"\n";
        return false;
    }
    return true;
}

std::string SchoolResolver::toLower(const std::string& s) {
    std::string r = s;
    std::transform(r.begin(), r.end(), r.begin(), ::tolower);
    return r;
}

size_t SchoolResolver::editDistance(const std::string& a, const std::string& b) {
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
