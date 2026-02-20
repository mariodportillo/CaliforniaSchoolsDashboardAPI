#ifndef SUMMARYCARD_H
#define SUMMARYCARD_H
#include <nlohmann/json.hpp>
#include <string>
#include <vector>
#include <map>

class SummaryCard {
    // A helpful map to use for lookup of indciatorIDs to category
    std::map<int, std::string> indicatorsMap = {
        {1, "CHRONIC_ABSENTEEISM"},
        {2, "SUSPENSION_RATE"},
        {3, "ENGLISH_LEARNER_PROGRESS"},
        {4, "GRADUATION_RATE"},
        {5, "COLLEGE_CAREER_INDICATOR"},
        {6, "ELA_POINTS_ABOVE_BELOW"},
        {7, "MATHEMATICS"},
        {8, "SCIENCE"}
    };
    // our indicator struct to hold all the following data. 
    struct indicator {
        std::string indicatorCategory;
        nlohmann::json primary;
        nlohmann::json secondary;
        std::string cdsCode;
        size_t indicatorId;
        float status;
        float change;
        int changeId;
        int statusId;
        int performance;
        size_t totalGroups;
        int red;
        int orange;
        int yellow;
        int green;
        int blue;
        long count;
        std::string studentGroup;
        size_t schoolYearId;
        bool isPrivateData;
    };

private:
    std::string rawData;
    nlohmann::json rawJsonData;
    std::vector<SummaryCard::indicator> indicatorVector;
    SummaryCard::indicator parseIndicator(const nlohmann::json& entry);
    std::vector<SummaryCard::indicator> parseIndicators(const nlohmann::json& data);
    std::map<std::string, SummaryCard::indicator> categoryToIndicatorMap;

public:
    // Metadata — school name and year this card was fetched for.
    // Set via setMetadata() after fetching.
    std::string schoolName;
    std::string year;

    // Constructors
    SummaryCard();
    explicit SummaryCard(const std::string& jsonString);

    // Destructor
    ~SummaryCard();

    // Data manipulation
    void setRawData(const std::string& data);
    void appendRawData(const char* data, size_t size);
    void parseRawData();
    void clear();

    // Stamps school name and year onto the card after fetching.
    void setMetadata(const std::string& school, const std::string& yr);

    // Getters
    const std::string& getRawData() const;
    const nlohmann::json& getRawJsonData() const;
    std::vector<SummaryCard::indicator> getIndicatorVector() const;
    const std::map<std::string, SummaryCard::indicator>& getCategoryMap() const;

    // Print
    bool printRawData() const;
    bool printIndicatorVector() const;

    // Export and import
    bool saveToFile(const std::string& filename) const;
    bool loadFromFile(const std::string& filename);
};

#endif // SUMMARYCARD_H
