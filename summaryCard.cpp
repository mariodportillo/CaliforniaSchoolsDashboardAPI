#include "summaryCard.hh"
#include <fstream>
#include <iostream>

// Constructors and Destructor
SummaryCard::SummaryCard(){
    rawData = ""; 
    rawJsonData = json::array();
}

SummaryCard::SummaryCard(const std::string& jsonString) : rawData(jsonString) {
    parseRawData();
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
      try { rawJsonData = json::parse(rawData);
      } catch (const json::parse_error& e) {
          std::cerr << "JSON Parse Error: " << e.what() << std::endl;
          rawJsonData = json::array();
      }
    }
}


void SummaryCard::clear() {
    rawData.clear();
    rawJsonData.clear();
}

// Getters
const std::string& SummaryCard::getRawData() const {
    return rawData;
}

const json& SummaryCard::getRawJsonData() const {
    return rawJsonData;
}

bool SummaryCard::printRawData() {
    if (rawData.empty()) {
        std::cerr << "Error: No raw data to print." << std::endl;
        return false;
    }

    std::cout << rawData << std::endl;
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
