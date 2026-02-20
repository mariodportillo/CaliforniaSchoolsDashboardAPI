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
    // Stable ceiling — 64 workers, 64 req/sec.
    // 50 was clean, 100 was too much. 64 splits the difference
    // and the light rate cap prevents burst pressure at startup.
    static constexpr std::size_t DEFAULT_POOL_SIZE            = 50;
    static constexpr double      DEFAULT_MAX_REQUESTS_PER_SEC = 1000.0;
    static constexpr long        DEFAULT_TIMEOUT_MS           = 10'000;

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

    struct PoolWorkerArg {
        CaliforniaDashboardAPI* self;
        WorkQueue*              queue;
        CURL*                   curl;    // persistent handle, one per worker
        std::size_t             slot;    // pre-allocated slot in results vector
    };

    static void*  poolWorker(void* raw);
    static size_t write_callback(char* ptr, size_t size, size_t nmemb, void* userdata);
    CURLcode      fetchSummaryCard(CURL* curl, const std::string& url, SummaryCard& card);
    void          acquireToken();

    long        timeout_ms_;
    std::size_t pool_size_;
    double      max_requests_per_sec_;

    // Token bucket — protected by rate_mutex_
    double          tokens_;
    struct timespec last_refill_;
    std::mutex      rate_mutex_;

    // Progress — separate from results mutex so printing never blocks a push
    std::atomic<std::size_t> completed_{0};
    std::size_t              total_{0};
    std::mutex               progress_mutex_; // serialises stderr writes only

    // Results — atomic slot index eliminates mutex on the hot path.
    // Vector is pre-sized before workers start; each worker writes to its
    // own slot via an atomic fetch_add, so no two workers ever touch the
    // same element and no lock is needed.
    std::atomic<std::size_t> next_slot_{0};
    std::mutex               results_mutex_; // only used for reserve()

    // Shared CURL state — DNS cache and SSL session shared across all handles
    // so the first worker to resolve the host shares the result with all others.
    CURLSH* curl_share_{nullptr};

    std::string              ca_bundle_path_; // resolved once at construction
    std::vector<std::string> urls_;
};

#endif // CALIFORNIADASHBOARDAPI_H
