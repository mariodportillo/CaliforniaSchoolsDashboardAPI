#include "summaryCard.hh"
#include <cstdio>
#include <curl/curl.h>
#include <iostream>

// CURL callback function
size_t write_callback(char *ptr, size_t size, size_t nmemb, void *userdata) {
    size_t bytes = size * nmemb;
    SummaryCard* card = static_cast<SummaryCard*>(userdata);
    card->appendRawData(ptr, bytes);
    std::cout << "Received chunk: " << bytes << " bytes" << std::endl;
    return bytes;
}

CURLcode fetchSummaryCard(const std::string& url, CURL *curl, SummaryCard& card) {
    if (!curl) {
        fprintf(stderr, "CURL initialization failed\n");
        return CURLE_FAILED_INIT;
    }
    
    curl_easy_setopt(curl, CURLOPT_URL, url.c_str());
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, write_callback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA, &card);
    curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 1L);
    
    CURLcode result = curl_easy_perform(curl);
    if (result != CURLE_OK) {
        fprintf(stderr, "CURL Error: %s\n", curl_easy_strerror(result));
    }
    
    curl_easy_cleanup(curl);
    return result;
}

int main() {
    // Initialize CURL
    curl_global_init(CURL_GLOBAL_DEFAULT);
    CURL *curl = curl_easy_init();
    
    // API endpoint
    std::string url = "https://api.caschooldashboard.org/Reports/19649071995901/11/SummaryCards";
    
    // Create SummaryCard object
    SummaryCard card;
    
    // Fetch data from API
    std::cout << "Fetching data from API..." << std::endl;
    CURLcode result = fetchSummaryCard(url, curl, card);
    
    if (result == CURLE_OK) {
        std::cout << "\nData fetched successfully!" << std::endl;
        std::cout << "Processing data..." << std::endl;
        
        // Process the raw data
        card.parseRawData();

        // Print rawData
        card.printRawData();

    } else {
        std::cerr << "Failed to fetch data from API" << std::endl;
    }
    
    // Cleanup
    curl_global_cleanup();
    
    return 0;
}
