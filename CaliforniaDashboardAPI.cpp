#include "CaliforniaDashboardAPI.hh"
#include <cstring>
#include <cstdio>
#include <stdexcept>
#include <time.h>

// =============================================================================
// Constructor / Destructor
// =============================================================================

CaliforniaDashboardAPI::CaliforniaDashboardAPI(long timeout_ms, std::size_t pool_size,
                                               double max_requests_per_sec)
    : timeout_ms_(timeout_ms),
      pool_size_(pool_size),
      max_requests_per_sec_(max_requests_per_sec),
      tokens_(max_requests_per_sec)
{
    CURLcode rc = curl_global_init(CURL_GLOBAL_ALL);
    if (rc != CURLE_OK)
        throw std::runtime_error(std::string("curl_global_init: ") +
                                 curl_easy_strerror(rc));
    clock_gettime(CLOCK_MONOTONIC, &last_refill_);
}

CaliforniaDashboardAPI::~CaliforniaDashboardAPI() {
    curl_global_cleanup();
}

// =============================================================================
// loadInURLs
// =============================================================================

bool CaliforniaDashboardAPI::loadInURLs(const std::vector<std::string>& urls)
{
    if (urls.empty()) {
        fprintf(stderr, "loadInURLs: provided URL list is empty\n");
        return false;
    }

    std::vector<std::string> valid;
    valid.reserve(urls.size());

    for (const auto& url : urls) {
        if (url.empty()) { fprintf(stderr, "loadInURLs: skipping empty URL\n"); continue; }
        if (url.rfind("https://", 0) != 0 &&
            url.rfind("http://",  0) != 0 &&
            url.rfind("ftp://",   0) != 0) {
            fprintf(stderr, "loadInURLs: skipping invalid URL: %s\n", url.c_str());
            continue;
        }
        valid.push_back(url);
    }

    if (valid.empty()) {
        fprintf(stderr, "loadInURLs: no valid URLs found in list\n");
        return false;
    }

    urls_.insert(urls_.end(),
                 std::make_move_iterator(valid.begin()),
                 std::make_move_iterator(valid.end()));
    return true;
}

// =============================================================================
// runFullURLFetch
// =============================================================================

bool CaliforniaDashboardAPI::runFullURLFetch()
{
    if (urls_.empty()) {
        fprintf(stderr, "runFullURLFetch: no URLs loaded — call loadInURLs first.\n");
        return false;
    }

    allSummaryCardsVector.reserve(allSummaryCardsVector.size() + urls_.size());
    total_     = urls_.size();
    completed_ = 0;

    // Fill work queue
    WorkQueue queue;
    {
        std::lock_guard<std::mutex> lk(queue.mtx);
        for (const auto& url : urls_)
            queue.items.push(url);
    }

    const std::size_t n = std::min(pool_size_, urls_.size());
    std::vector<pthread_t>     tids(n);
    std::vector<PoolWorkerArg> args(n);

    // Initialise one persistent CURL handle per worker BEFORE spawning.
    // Setting options here means they are inherited for every request the
    // worker makes — headers, keep-alive, DNS cache, etc. are set once.
    for (std::size_t i = 0; i < n; ++i) {
        CURL* curl = curl_easy_init();
        if (!curl) {
            fprintf(stderr, "runFullURLFetch: curl_easy_init failed for worker %zu\n", i);
            // Clean up handles already created
            for (std::size_t j = 0; j < i; ++j) curl_easy_cleanup(args[j].curl);
            return false;
        }

        // -- Browser identity -------------------------------------------------
        // Spoofing a real browser User-Agent causes the server to treat
        // requests the same as dashboard website traffic, bypassing
        // aggressive connection throttling aimed at bots.
        curl_easy_setopt(curl, CURLOPT_USERAGENT,
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
            "AppleWebKit/537.36 (KHTML, like Gecko) "
            "Chrome/120.0.0.0 Safari/537.36");

        struct curl_slist* headers = nullptr;
        headers = curl_slist_append(headers, "Referer: https://www.caschooldashboard.org/");
        headers = curl_slist_append(headers, "Accept: application/json, text/plain, */*");
        headers = curl_slist_append(headers, "Accept-Language: en-US,en;q=0.9");
        headers = curl_slist_append(headers, "Connection: keep-alive");
        curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);
        // Note: headers list must outlive the handle. We free it in poolWorker.

        // -- Connection reuse -------------------------------------------------
        // TCP keep-alive probes the connection every 30s so the OS doesn't
        // close idle sockets between sequential requests from this worker.
        curl_easy_setopt(curl, CURLOPT_TCP_KEEPALIVE, 1L);
        curl_easy_setopt(curl, CURLOPT_TCP_KEEPIDLE,  30L);
        curl_easy_setopt(curl, CURLOPT_TCP_KEEPINTVL, 15L);

        // Keep the DNS result cached for 120s to avoid re-resolving the same
        // host on every request (default is only 60s).
        curl_easy_setopt(curl, CURLOPT_DNS_CACHE_TIMEOUT, 120L);

        // Follow redirects and set timeout
        curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 1L);
        curl_easy_setopt(curl, CURLOPT_TIMEOUT_MS,     timeout_ms_);

        // Write callback — overridden per-request with CURLOPT_WRITEDATA
        curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, &CaliforniaDashboardAPI::write_callback);

        args[i] = { this, &queue, curl };
    }

    // Spawn workers
    std::size_t spawned = 0;
    for (std::size_t i = 0; i < n; ++i) {
        int err = pthread_create(&tids[i], nullptr, &CaliforniaDashboardAPI::poolWorker, &args[i]);
        if (err) {
            fprintf(stderr, "runFullURLFetch: pthread_create failed: %s\n", strerror(err));
            {
                std::lock_guard<std::mutex> lk(queue.mtx);
                queue.done = true;
            }
            queue.cv.notify_all();
            for (std::size_t j = 0; j < spawned; ++j) pthread_join(tids[j], nullptr);
            for (std::size_t j = 0; j < n; ++j)       curl_easy_cleanup(args[j].curl);
            return false;
        }
        ++spawned;
    }

    // Signal no more work will be enqueued
    {
        std::lock_guard<std::mutex> lk(queue.mtx);
        queue.done = true;
    }
    queue.cv.notify_all();

    for (std::size_t i = 0; i < spawned; ++i)
        pthread_join(tids[i], nullptr);

    // Clean up persistent handles after all threads have exited
    for (std::size_t i = 0; i < n; ++i)
        curl_easy_cleanup(args[i].curl);

    return true;
}

// =============================================================================
// poolWorker
// =============================================================================

void* CaliforniaDashboardAPI::poolWorker(void* raw)
{
    auto* a  = static_cast<PoolWorkerArg*>(raw);
    WorkQueue& q = *a->queue;

    while (true) {
        std::string url;

        {
            std::unique_lock<std::mutex> lk(q.mtx);
            q.cv.wait(lk, [&q] { return !q.items.empty() || q.done; });
            if (q.items.empty()) break;
            url = std::move(q.items.front());
            q.items.pop();
        }

        // Throttle globally before firing the request
        a->self->acquireToken();

        // Fetch using this worker's persistent CURL handle
        SummaryCard card;
        a->self->fetchSummaryCard(a->curl, url, card);

        {
            std::lock_guard<std::mutex> lk(a->self->mutex_);
            a->self->allSummaryCardsVector.push_back(std::move(card));
        }

        // Progress bar
        {
            std::size_t done  = ++a->self->completed_;
            std::size_t total = a->self->total_;
            int pct    = static_cast<int>(done * 100 / total);
            int filled = pct / 2;

            fprintf(stderr, "\r  [");
            for (int i = 0; i < 50; ++i)
                fputc(i < filled ? '#' : '-', stderr);
            fprintf(stderr, "] %3d%%  %zu/%zu", pct, done, total);
            fflush(stderr);

            if (done == total) fputc('\n', stderr);
        }
    }

    return nullptr;
}

// =============================================================================
// write_callback
// =============================================================================

size_t CaliforniaDashboardAPI::write_callback(char* ptr, size_t size,
                                               size_t nmemb, void* userdata)
{
    size_t bytes = size * nmemb;
    static_cast<SummaryCard*>(userdata)->appendRawData(ptr, bytes);
    return bytes;
}

// =============================================================================
// acquireToken  —  global token bucket rate limiter
// =============================================================================

void CaliforniaDashboardAPI::acquireToken()
{
    std::unique_lock<std::mutex> lk(rate_mutex_);

    while (true) {
        struct timespec now;
        clock_gettime(CLOCK_MONOTONIC, &now);

        double elapsed = (now.tv_sec  - last_refill_.tv_sec) +
                         (now.tv_nsec - last_refill_.tv_nsec) / 1e9;

        tokens_ += elapsed * max_requests_per_sec_;
        if (tokens_ > max_requests_per_sec_)
            tokens_ = max_requests_per_sec_;
        last_refill_ = now;

        if (tokens_ >= 1.0) {
            tokens_ -= 1.0;
            return;
        }

        double wait_sec = (1.0 - tokens_) / max_requests_per_sec_;
        long   wait_ns  = static_cast<long>(wait_sec * 1e9);
        struct timespec ts = { wait_ns / 1'000'000'000L, wait_ns % 1'000'000'000L };

        lk.unlock();
        nanosleep(&ts, nullptr);
        lk.lock();
    }
}

// =============================================================================
// fetchSummaryCard  —  uses caller-supplied persistent CURL handle
// =============================================================================

static bool isRetryable(CURLcode code) {
    switch (code) {
        case CURLE_OPERATION_TIMEDOUT:
        case CURLE_COULDNT_RESOLVE_HOST:
        case CURLE_COULDNT_CONNECT:
        case CURLE_RECV_ERROR:
        case CURLE_SEND_ERROR:
        case CURLE_GOT_NOTHING:
            return true;
        default:
            return false;
    }
}

CURLcode CaliforniaDashboardAPI::fetchSummaryCard(CURL*              curl,
                                                   const std::string& url,
                                                   SummaryCard&       card)
{
    static constexpr int  MAX_RETRIES   = 4;
    static constexpr long BASE_DELAY_MS = 500; // doubles each retry

    // Point this handle at the new URL and new card buffer.
    // All other options (headers, keep-alive, etc.) are already set.
    curl_easy_setopt(curl, CURLOPT_URL,       url.c_str());
    curl_easy_setopt(curl, CURLOPT_WRITEDATA, &card);

    CURLcode result = CURLE_OK;

    for (int attempt = 0; attempt <= MAX_RETRIES; ++attempt) {
        if (attempt > 0) {
            card.clear();
            long delay_ms = BASE_DELAY_MS * (1L << (attempt - 1));
            fprintf(stderr, "[RETRY %d/%d] +%ldms  %s\n",
                    attempt, MAX_RETRIES, delay_ms, url.c_str());
            struct timespec ts = { delay_ms / 1000, (delay_ms % 1000) * 1'000'000L };
            nanosleep(&ts, nullptr);
        }

        result = curl_easy_perform(curl);
        if (result == CURLE_OK) break;

        fprintf(stderr, "CURL Error (attempt %d/%d) [%s]: %s\n",
                attempt + 1, MAX_RETRIES + 1, url.c_str(), curl_easy_strerror(result));
        if (!isRetryable(result)) break;
    }

    if (result != CURLE_OK) return result;

    long http_code = 0;
    curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &http_code);
    if (http_code < 200 || http_code >= 300) {
        fprintf(stderr, "HTTP Error [%ld] for URL: %s\n", http_code, url.c_str());
        return CURLE_HTTP_RETURNED_ERROR;
    }

    const std::string& raw = card.getRawData();
    if (raw.empty()) {
        fprintf(stderr, "Empty response for URL: %s\n", url.c_str());
        return CURLE_GOT_NOTHING;
    }

    size_t first = raw.find_first_not_of(" \t\r\n");
    if (first == std::string::npos || (raw[first] != '{' && raw[first] != '[')) {
        fprintf(stderr, "Invalid JSON for URL: %s\nPreview: %.200s\n", url.c_str(), raw.c_str());
        return CURLE_GOT_NOTHING;
    }

    card.parseRawData();
    return result;
}
