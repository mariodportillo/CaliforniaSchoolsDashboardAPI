#include "summaryCard.hh"
#include <fstream>
#include <iostream>

using json = nlohmann::json;

// =============================================================================
// Constructors and Destructor
// =============================================================================

SummaryCard::SummaryCard(){
    // init empty data
    rawData     = "";
    rawJsonData = json::array();
}

SummaryCard::SummaryCard(const std::string& jsonString) : rawData(jsonString) {
    // When provided valid jsonString we can now parse the data
    parseRawData();
    
    // We also parseIndicators here as well as outlined by our indicator struct  
    indicatorVector = parseIndicators(rawJsonData);
}

SummaryCard::~SummaryCard() {
    clear();
}

// =============================================================================
// Data Manipulation
// =============================================================================

void SummaryCard::setRawData(const std::string& data) {
    rawData = data;
}

void SummaryCard::appendRawData(const char* data, size_t size) {
    rawData.append(data, size);
}

void SummaryCard::parseRawData() {
    if (rawData.empty()) return;
    // we safely go in and try to parseRawData input 
    try {
        rawJsonData     = json::parse(rawData);
        // we need to parse the indicators themselves
        indicatorVector = parseIndicators(rawJsonData);
    }
    catch (const json::parse_error& e) {
        std::cerr << "JSON Parse Error: " << e.what() << std::endl;
        rawJsonData = json::array();
    }
}

void SummaryCard::clear() {
    rawData.clear();
    rawJsonData.clear();
    indicatorsMap.clear();
    indicatorVector.clear();
    categoryToIndicatorMap.clear();
    schoolName.clear();
    year.clear();
}
// This helpful function will help make sure we can assign a card a schoolName and year. 
// This can only be done by the caller since the summary cards and json do not have 
// this information within the data. 
void SummaryCard::setMetadata(const std::string& school, const std::string& yr) {
    schoolName = school;
    year       = yr;
}

// =============================================================================
// Getters
// =============================================================================

const std::string& SummaryCard::getRawData() const {
    return rawData;
}

const json& SummaryCard::getRawJsonData() const {
    return rawJsonData;
}

std::vector<SummaryCard::indicator> SummaryCard::getIndicatorVector() const {
    return indicatorVector;
}

const std::map<std::string, SummaryCard::indicator>& SummaryCard::getCategoryMap() const {
    return categoryToIndicatorMap;
}

// =============================================================================
// Print / Save / Load
// =============================================================================

bool SummaryCard::printRawData() const {
    if (rawData.empty()) {
        std::cerr << "Error: No raw data to print." << std::endl;
        return false;
    }
    std::cout << rawData << std::endl;
    return true;
}

bool SummaryCard::printIndicatorVector() const {
    if (indicatorVector.empty()) {
        std::cerr << "Error: No indicator data to print." << std::endl;
        return false;
    }

    // Print card-level metadata if available
    if (!schoolName.empty() || !year.empty()) {
        std::cout << "=============================\n"
                  << "School: " << (schoolName.empty() ? "Unknown" : schoolName) << "\n"
                  << "Year:   " << (year.empty()       ? "Unknown" : year)       << "\n"
                  << "=============================\n";
    }

    for (const auto& ind : indicatorVector) {
        std::cout << "-----------------------------\n"
                  << "Category:     " << ind.indicatorCategory << "\n"
                  << "CDS Code:     " << ind.cdsCode           << "\n"
                  << "Indicator ID: " << ind.indicatorId       << "\n"
                  << "Status:       " << ind.status            << "\n"
                  << "Change:       " << ind.change            << "\n"
                  << "Status ID:    " << ind.statusId          << "\n"
                  << "Performance:  " << ind.performance       << "\n"
                  << "Total Groups: " << ind.totalGroups       << "\n"
                  << "Count:        " << ind.count             << "\n"
                  << "Student Group:" << ind.studentGroup      << "\n"
                  << "Colors:       "
                  << "R=" << ind.red    << " "
                  << "O=" << ind.orange << " "
                  << "Y=" << ind.yellow << " "
                  << "G=" << ind.green  << " "
                  << "B=" << ind.blue   << "\n"
                  << "Private Data: " << std::boolalpha << ind.isPrivateData << "\n";
    }
    return true;
}

bool SummaryCard::saveToFile(const std::string& filename) const {
    std::ofstream file(filename);
    if (!file.is_open()) {
        std::cerr << "Error: Could not open file for writing: " << filename << std::endl;
        return false;
    }
    file << rawJsonData;
    if (file.fail()) {
        std::cerr << "Error: Failed to write to file: " << filename << std::endl;
        return false;
    }
    file.close();
    return true;
}

bool SummaryCard::loadFromFile(const std::string& filename) {
    std::ifstream file(filename);
    if (!file.is_open()) {
        std::cerr << "Error: Could not open file for reading: " << filename << std::endl;
        return false;
    }
    std::string content((std::istreambuf_iterator<char>(file)),
                         std::istreambuf_iterator<char>());
    file.close();
    setRawData(content);
    parseRawData();
    return true;
}

// =============================================================================
// Parsing Helpers
// =============================================================================

static std::string safeString(const json& obj, const std::string& key,
                               const std::string& defaultVal = "") {
    if (!obj.contains(key) || obj[key].is_null()) return defaultVal;
    if (obj[key].is_string()) return obj[key].get<std::string>();
    return obj[key].dump();
}

static int safeInt(const json& obj, const std::string& key, int defaultVal = 0) {
    if (!obj.contains(key) || obj[key].is_null()) return defaultVal;
    if (obj[key].is_number()) return obj[key].get<int>();
    return defaultVal;
}

static long safeLong(const json& obj, const std::string& key, long defaultVal = 0L) {
    if (!obj.contains(key) || obj[key].is_null()) return defaultVal;
    if (obj[key].is_number()) return obj[key].get<long>();
    return defaultVal;
}

static size_t safeSizeT(const json& obj, const std::string& key, size_t defaultVal = 0) {
    if (!obj.contains(key) || obj[key].is_null()) return defaultVal;
    if (obj[key].is_number_unsigned()) return obj[key].get<size_t>();
    if (obj[key].is_number())          return static_cast<size_t>(obj[key].get<long>());
    return defaultVal;
}

static float safeFloat(const json& obj, const std::string& key, float defaultVal = 0.0f) {
    if (!obj.contains(key) || obj[key].is_null()) return defaultVal;
    if (obj[key].is_number()) return obj[key].get<float>();
    return defaultVal;
}

static bool safeBool(const json& obj, const std::string& key, bool defaultVal = false) {
    if (!obj.contains(key) || obj[key].is_null()) return defaultVal;
    if (obj[key].is_boolean()) return obj[key].get<bool>();
    return defaultVal;
}
// The following is used to build an indicator from the json we get from the California Dashboard
SummaryCard::indicator SummaryCard::parseIndicator(const json& entry) {
    indicator ind;

    // We return an empty indicator if we do not have valid json
    if (!entry.is_object()) {
        std::cerr << "[WARN] parseIndicator: entry is not a JSON object, skipping.\n";
        return ind;
    }
    // We go ahead and use safe parsers to ensure we properly populate the indicator data
    ind.indicatorId = safeSizeT(entry, "indicatorId");

    auto it = indicatorsMap.find(static_cast<int>(ind.indicatorId));
    ind.indicatorCategory = (it != indicatorsMap.end()) ? it->second : "UNKNOWN";

    ind.primary   = entry.contains("primary")   ? entry["primary"]   : json(nullptr);
    ind.secondary = entry.contains("secondary") ? entry["secondary"] : json(nullptr);

    const json& p = ind.primary;
    // we build our indicator and populate all of our fields
    if (!p.is_null() && p.is_object()) {
        ind.cdsCode       = safeString(p, "cdsCode");
        ind.status        = safeFloat (p, "status");
        ind.change        = safeFloat (p, "change");
        ind.changeId      = safeInt   (p, "changeId");
        ind.statusId      = safeInt   (p, "statusId");
        ind.performance   = safeInt   (p, "performance");
        ind.totalGroups   = safeSizeT (p, "totalGroups");
        ind.red           = safeInt   (p, "red");
        ind.orange        = safeInt   (p, "orange");
        ind.yellow        = safeInt   (p, "yellow");
        ind.green         = safeInt   (p, "green");
        ind.blue          = safeInt   (p, "blue");
        ind.count         = safeLong  (p, "count");
        ind.studentGroup  = safeString(p, "studentGroup");
        ind.schoolYearId  = safeSizeT (p, "schoolYearId");
        ind.isPrivateData = safeBool  (p, "isPrivateData");
    } else {
        std::cerr << "[WARN] parseIndicator: missing or null 'primary' block "
                  << "for indicatorId=" << ind.indicatorId << "\n";
    }

    return ind;
}

// Use this function to go through every indicator withing a given SummaryCard json format. 
std::vector<SummaryCard::indicator> SummaryCard::parseIndicators(const json& data) {
    std::vector<indicator> result;

    if (data.is_null()) return result;

    const json& arr = data.is_array() ? data : json::array({data});
    result.reserve(arr.size());

    for (const auto& entry : arr) {
        if (!entry.is_object()) {
            std::cerr << "[WARN] parseIndicators: skipping non-object entry.\n";
            continue;
        }
        indicator ind = parseIndicator(entry);
        categoryToIndicatorMap[ind.indicatorCategory] = ind;
        result.push_back(std::move(ind));
    }

    return result;
}
