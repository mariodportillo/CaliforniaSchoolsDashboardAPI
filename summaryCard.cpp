#include "summaryCard.hh"
#include <fstream>
#include <iostream>

using json = nlohmann::json;

// Constructors and Destructor
SummaryCard::SummaryCard(){
    rawData = ""; 
    rawJsonData = json::array();
}

SummaryCard::SummaryCard(const std::string& jsonString) : rawData(jsonString) {
    parseRawData();
    indicatorVector = parseIndicators(rawJsonData);
}

SummaryCard::~SummaryCard() {
    clear();
}

// Data Manipulation Methods
void SummaryCard::setRawData(const std::string& data) {
    rawData = data;
}

void SummaryCard::appendRawData(const char* data, size_t size) {
    rawData.append(data, size);
}

void SummaryCard::parseRawData() {
    if (!rawData.empty()) {
        try { 
            rawJsonData = json::parse(rawData);
            indicatorVector = parseIndicators(rawJsonData);
        } 
        catch (const json::parse_error& e){
            std::cerr << "JSON Parse Error: " << e.what() << std::endl;
            rawJsonData = json::array();
        }
    }
}


void SummaryCard::clear() {
    rawData.clear();
    rawJsonData.clear();
    indicatorsMap.clear();
    indicatorVector.clear();
}

// Getters
const std::string& SummaryCard::getRawData() const {
    return rawData;
}

const json& SummaryCard::getRawJsonData() const {
    return rawJsonData;
}

std::vector<SummaryCard::indicator> SummaryCard::getIndicatorVector() const{
    return indicatorVector;
}


const std::map<std::string, SummaryCard::indicator>& SummaryCard::getCategoryMap() const{
    return categoryToIndicatorMap;
}

// Print outs
bool SummaryCard::printRawData() const{
    if (rawData.empty()) {
        std::cerr << "Error: No raw data to print." << std::endl;
        return false;
    }

    std::cout << rawData << std::endl;
    return true;
}


bool SummaryCard::printIndicatorVector() const{
    if (indicatorVector.empty()) {
        std::cerr << "Error: No indicator data to print." << std::endl;
        return false;
    }
    for (const auto& ind : indicatorVector) {
        std::cout << "-----------------------------\n"
                  << "Category:    " << ind.indicatorCategory << "\n"
                  << "CDS Code:    " << ind.cdsCode           << "\n"
                  << "Indicator ID:" << ind.indicatorId       << "\n"
                  << "Status:      " << ind.status            << "\n"
                  << "Change:      " << ind.change            << "\n"
                  << "Status ID:   " << ind.statusId          << "\n"
                  << "Performance: " << ind.performance       << "\n"
                  << "Total Groups:" << ind.totalGroups       << "\n"
                  << "Count:       " << ind.count             << "\n"
                  << "Student Group:" << ind.studentGroup     << "\n"
                  << "Colors:      "
                  << "R=" << ind.red    << " "
                  << "O=" << ind.orange << " "
                  << "Y=" << ind.yellow << " "
                  << "G=" << ind.green  << " "
                  << "B=" << ind.blue   << "\n"
                  << "Private Data:" << std::boolalpha << ind.isPrivateData << "\n";
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
    if (file.fail()) {
        std::cerr << "Error: Failed to close file: " << filename << std::endl;
        return false;
    }

    return true;
}

SummaryCard::indicator SummaryCard::parseIndicator(const json& entry) {
    indicator ind;
    const auto& p = entry["primary"];

    // Top-level
    ind.indicatorId = entry["indicatorId"];

    // Look up category from your map
    auto it = indicatorsMap.find(ind.indicatorId);
    ind.indicatorCategory = (it != indicatorsMap.end()) 
                            ? it->second 
                            : "UNKNOWN";

    // Store both raw objects for later secondary comparisons
    ind.primary   = entry["primary"];
    ind.secondary = entry["secondary"];

    // Flat fields from primary
    ind.cdsCode       = p["cdsCode"];
    ind.status        = p["status"];
    ind.change        = p["change"];
    ind.changeId      = p["changeId"];
    ind.statusId      = p["statusId"];
    ind.performance   = p["performance"];
    ind.totalGroups   = p["totalGroups"];
    ind.red           = p["red"];
    ind.orange        = p["orange"];
    ind.yellow        = p["yellow"];
    ind.green         = p["green"];
    ind.blue          = p["blue"];
    ind.count         = p["count"];
    ind.studentGroup  = p["studentGroup"];
    ind.schoolYearId  = p["schoolYearId"];
    ind.isPrivateData = p["isPrivateData"];

    return ind;
}

std::vector<SummaryCard::indicator> SummaryCard::parseIndicators(const json& data) {
    std::vector<indicator> result;
    result.reserve(data.size());

    for (const auto& entry : data) {
        indicator ind = parseIndicator(entry);
        categoryToIndicatorMap[ind.indicatorCategory] = ind; // ← populate map
        result.push_back(ind);
    }

    return result;
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
