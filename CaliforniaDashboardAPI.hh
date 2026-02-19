#ifndef CALIFORNIADASHBOARDAPI_H
#define CALIFORNIADASHBOARDAPI_H

#include "summaryCard.hh"
#include <curl/curl.h>
#include <pthread.h>
#include <mutex>
#include <condition_variable>
#include <atomic>
#include <queue>
#include <string>
#include <vector>

class CaliforniaDashboardAPI {
public:
    // -- Tuning knobs ---------------------------------------------------------
    // pool_size:            number of persistent worker threads.
    //                       Each worker owns one CURL handle and reuses its
    //                       TCP/TLS connection, so more workers = more
    //                       parallel connections to the server.
    //
    // max_requests_per_sec: global token-bucket cap across ALL workers.
    //                       The server was refusing connections at >8 req/sec
    //                       without browser headers; with them it tolerates
    //                       much higher rates. Start at 20 and back off if
    //                       you see connection errors.
    //
    // timeout_ms:           per-request timeout. 30s is generous; the API
    //                       usually responds in <2s on a live connection.
    static constexpr std::size_t DEFAULT_POOL_SIZE            = 20;
    static constexpr double      DEFAULT_MAX_REQUESTS_PER_SEC = 20.0;
    static constexpr long        DEFAULT_TIMEOUT_MS           = 30'000;

    explicit CaliforniaDashboardAPI(
        long        timeout_ms           = DEFAULT_TIMEOUT_MS,
        std::size_t pool_size            = DEFAULT_POOL_SIZE,
        double      max_requests_per_sec = DEFAULT_MAX_REQUESTS_PER_SEC);

    ~CaliforniaDashboardAPI();

    bool loadInURLs(const std::vector<std::string>& urls);
    bool runFullURLFetch();

    std::vector<SummaryCard> allSummaryCardsVector;

private:
    // -- Work queue -----------------------------------------------------------
    struct WorkQueue {
        std::queue<std::string> items;
        std::mutex              mtx;
        std::condition_variable cv;
        bool                    done = false;
    };

    // Each worker carries its own persistent CURL handle so TCP/TLS
    // connections are reused across requests to the same host.
    struct PoolWorkerArg {
        CaliforniaDashboardAPI* self;
        WorkQueue*              queue;
        CURL*                   curl; // owned by this worker, init'd before spawn
    };

    static void* poolWorker(void* raw);

    // -- Per-request fetch (takes a pre-initialised CURL handle) --------------
    static size_t write_callback(char* ptr, size_t size, size_t nmemb, void* userdata);
    CURLcode fetchSummaryCard(CURL* curl, const std::string& url, SummaryCard& card);

    // -- Token bucket rate limiter --------------------------------------------
    void acquireToken();

    long        timeout_ms_;
    std::size_t pool_size_;
    double      max_requests_per_sec_;

    double          tokens_;
    struct timespec last_refill_;
    std::mutex      rate_mutex_;

    std::atomic<std::size_t> completed_{0};
    std::size_t              total_{0};

    std::vector<std::string> urls_;
    std::mutex               mutex_; // guards allSummaryCardsVector
};

#endif // CALIFORNIADASHBOARDAPI_H
