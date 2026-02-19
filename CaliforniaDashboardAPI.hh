#ifndef CALIFORNIADASHBOARDAPI_H
#define CALIFORNIADASHBOARDAPI_H

#include "summaryCard.hh"
#include <curl/curl.h>
#include <pthread.h>
#include <mutex>
#include <string>
#include <vector>

class CaliforniaDashboardAPI {

public:
    explicit CaliforniaDashboardAPI(long timeout_ms = 10'000);  // only one constructor
    ~CaliforniaDashboardAPI();

    bool loadInURLs(const std::vector<std::string> &urls);
    bool runFullURLFetch();

    std::vector<SummaryCard> allSummaryCardsVector;

private:
    struct ThreadArg {                  // needed to pass data into each thread
        CaliforniaDashboardAPI *self;
        std::string             url;
    };

    static void  *threadWorker(void *raw);          // must be static
    static size_t write_callback(char *ptr, size_t size, size_t nmemb, void *userdata);

    CURLcode fetchSummaryCard(const std::string &url, SummaryCard &card);

    long                     timeout_ms_;
    std::vector<std::string> urls_;
    std::mutex               mutex_;
};

#endif // CALIFORNIADASHBOARDAPI_H
