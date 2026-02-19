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
    // Max concurrent HTTP requests. Tune this to taste:
    //   64  — safe default, works on most systems
    //   128 — faster, requires ulimit -n > 256
    //   256 — aggressive, verify your fd limit first: `ulimit -n`
    // Tune these to avoid rate-limiting from the API server.
    // Lower pool_size = fewer simultaneous connections.
    // Higher request_delay_ms = more breathing room between each worker's fetches.

    explicit CaliforniaDashboardAPI(long        timeout_ms           = 30'000,
                                    std::size_t pool_size            = DEFAULT_POOL_SIZE,
                                    long        request_delay_ms     = DEFAULT_REQUEST_DELAY_MS,
                                    double      max_requests_per_sec = 8.0);
    static constexpr std::size_t DEFAULT_POOL_SIZE        = 20;
    static constexpr long        DEFAULT_REQUEST_DELAY_MS = 100; // ms between fetches per worker
    ~CaliforniaDashboardAPI();

    bool loadInURLs(const std::vector<std::string>& urls);

    // Fetches all loaded URLs using a bounded thread pool (pool_size_ workers).
    // Workers pull URLs from a shared queue until it is drained, then exit.
    bool runFullURLFetch();

    std::vector<SummaryCard> allSummaryCardsVector;

private:
    // -- Thread pool internals ------------------------------------------------
    struct WorkQueue {
        std::queue<std::string> items;
        std::mutex              mtx;
        std::condition_variable cv;
        bool                    done = false; // signals no more work will be enqueued
    };

    struct PoolWorkerArg {
        CaliforniaDashboardAPI* self;
        WorkQueue*              queue;
    };

    static void* poolWorker(void* raw);

    // -- Fetch helpers (used by pool workers) ---------------------------------
    static size_t write_callback(char* ptr, size_t size, size_t nmemb, void* userdata);
    CURLcode fetchSummaryCard(const std::string& url, SummaryCard& card);

    // -- Token bucket rate limiter -------------------------------------------
    // All workers call acquireToken() before each fetch. If the bucket is empty
    // they block until a token is available, capping global throughput at
    // max_requests_per_sec_ regardless of pool size.
    void acquireToken();

    long        timeout_ms_;
    std::size_t pool_size_;
    long        request_delay_ms_;
    double      max_requests_per_sec_;

    // Token bucket state
    double                  tokens_;          // current available tokens
    struct timespec         last_refill_;     // last time tokens were added
    std::mutex              rate_mutex_;      // guards token bucket state
    std::condition_variable rate_cv_;

    std::atomic<std::size_t> completed_{0}; // tracks fetch progress
    std::size_t              total_{0};       // total URLs to fetch

    std::vector<std::string> urls_;
    std::mutex               mutex_; // guards allSummaryCardsVector
};

#endif // CALIFORNIADASHBOARDAPI_H
