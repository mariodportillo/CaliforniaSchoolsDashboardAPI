#include "summaryCard.hh"
#include "CaliforniaDashboardAPI.hh"
#include <iostream>

int main()
{
    CaliforniaDashboardAPI api;

    std::vector<std::string> urls = {
        "https://api.caschooldashboard.org/Reports/19649071995901/11/SummaryCards"
    };

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

    for (const auto &card : api.allSummaryCardsVector) {
        std::cout << "\n=== Card ===" << std::endl;
        card.printRawData();
        card.printIndicatorVector();
    }

    return 0;
}
