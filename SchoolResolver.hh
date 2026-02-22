#ifndef SCHOOLRESOLVER_HH
#define SCHOOLRESOLVER_HH

#include <sqlite3.h>
#include <string>
#include <vector>
#include <map>
#include <unordered_map>
#include "summaryCard.hh"



/**
 * SchoolResolver
 *
 * Responsibilities:
 *   - Load active schools from pubschls.db (SQLite)
 *   - Resolve school names to CDS codes via exact / substring / fuzzy matching
 *   - Build CA Dashboard API URLs for a given set of schools + years
 *   - Build a "fetch all active schools" schools map
 *
 * Usage:
 *   SchoolResolver resolver("pubschls.db");
 *   auto schools = resolver.buildAllSchoolsMap({"2022", "2023"});
 *   auto [urls, metadata] = resolver.buildURLs(schools);
 */
class SchoolResolver {
public:
    // -------------------------------------------------------------------------
    // Types
    // -------------------------------------------------------------------------

    // schoolName -> list of year strings, e.g. { "Lincoln High" -> {"2022","2023"} }
    using SchoolsMap = std::map<std::string, std::vector<std::string>>;

    // URL -> (schoolName, year)
    using URLMetadata = std::map<std::string, std::pair<std::string, std::string>>;

    struct BuildResult {
        std::vector<std::string> urls;
        URLMetadata              metadata;
    };

    // -------------------------------------------------------------------------
    // Construction
    // -------------------------------------------------------------------------

    /**
     * Opens the SQLite database at dbPath and pre-loads the CDS lookup table.
     * Throws std::runtime_error if the database cannot be opened.
     */
    explicit SchoolResolver(const std::string& dbPath = "../pubschls.db");
    ~SchoolResolver();

    // Non-copyable (owns a sqlite3* handle)
    SchoolResolver(const SchoolResolver&)            = delete;
    SchoolResolver& operator=(const SchoolResolver&) = delete;

    // -------------------------------------------------------------------------
    // Public API
    // -------------------------------------------------------------------------

    /**
     * Returns every active school from the DB as a SchoolsMap, with each school
     * assigned the provided years.  Duplicate names get a "(CDSCode)" suffix so
     * both entries survive.
     */
    SchoolsMap buildAllSchoolsMap(const std::vector<std::string>& years) const;

    /**
     * For every (school, year) pair in `schools`, resolves a CDS code and
     * constructs a CA Dashboard API URL.  Returns both the URL list and the
     * metadata map needed to label SummaryCards after fetching.
     *
     * Unrecognised school names and unsupported years are skipped with a warning.
     */
    BuildResult buildURLs(const SchoolsMap& schools) const;

    /**
     * Convenience overload: builds the schools map internally then calls buildURLs.
     */
    BuildResult buildURLs(const std::vector<std::string>& years) const;

private:
    // -------------------------------------------------------------------------
    // Internal helpers
    // -------------------------------------------------------------------------

    void loadCDSLookup();

    std::string findBestMatch(const std::string& schoolName) const;

    static bool        validateYear(const std::string& year);
    static std::string toLower(const std::string& s);
    static size_t      editDistance(const std::string& a, const std::string& b);

    // -------------------------------------------------------------------------
    // Data
    // -------------------------------------------------------------------------

    sqlite3*    db_{ nullptr };
    std::string dbPath_;

    // key: lowercase school name  ->  value: CDSCode
    std::unordered_map<std::string, std::string> cdsLookup_;

    // key: lowercase school name  ->  value: original-case school name
    std::unordered_map<std::string, std::string> originalNames_;

    static constexpr size_t      MAX_EDIT_DISTANCE = 5;
    static constexpr const char* BASE_URL          = "https://api.caschooldashboard.org/Reports/";

    static const std::map<std::string, std::string> YEAR_TO_ID;
};

void enrichCardsWithMetadata(
    std::vector<SummaryCard>& cards,
    const SchoolResolver::URLMetadata& urlMetadata);
#endif // SCHOOLRESOLVER_HH
