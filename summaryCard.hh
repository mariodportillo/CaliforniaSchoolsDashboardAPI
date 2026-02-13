#ifndef SUMMARYCARD_H
#define SUMMARYCARD_H

#include <nlohmann/json.hpp>
#include <string>
#include <vector>

using json = nlohmann::json;

// Main SummaryCard class
class SummaryCard {
private:
    std::string rawData;
    json rawJsonData;
    
public:
    // Constructors
    SummaryCard();
    explicit SummaryCard(const std::string& jsonString);
    
    // Destructor
    ~SummaryCard();
    
    // Data manipulation methods
    void setRawData(const std::string& data);
    void appendRawData(const char* data, size_t size);
    void parseRawData();
    void clear();
    
    // Getters
    const std::string& getRawData() const;
    const json& getRawJsonData() const;
    
    // print
    bool printRawData();

    // Export methods
    bool saveToFile(const std::string& filename) const;
    bool loadFromFile(const std::string& filename);
};

#endif // SUMMARYCARD_H
