#include "CaliforniaDashboardAPI.hh"
#include <cstring>
#include <time.h>
#include <cstdio>
#include <stdexcept>

// =============================================================================
// Constructor / Destructor
// =============================================================================

CaliforniaDashboardAPI::CaliforniaDashboardAPI(long timeout_ms, std::size_t pool_size,
                                               long request_delay_ms, double max_requests_per_sec)
    : timeout_ms_(timeout_ms), pool_size_(pool_size), request_delay_ms_(request_delay_ms),
      max_requests_per_sec_(max_requests_per_sec), tokens_(max_requests_per_sec)
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
        if (url.empty()) {
            fprintf(stderr, "loadInURLs: skipping empty URL\n");
            continue;
        }
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
// runFullURLFetch  —  bounded thread pool
// =============================================================================

/**
 * Spawns pool_size_ persistent worker threads. Each worker blocks on the
 * shared WorkQueue, pops a URL, fetches it, and loops until the queue is
 * drained and marked done. This caps the number of simultaneous open sockets
 * and DNS lookups, preventing the "Couldn't resolve host" failures caused by
 * spawning tens of thousands of threads at once.
 */
bool CaliforniaDashboardAPI::runFullURLFetch()
{
    if (urls_.empty()) {
        fprintf(stderr, "runFullURLFetch: no URLs loaded — call loadInURLs first.\n");
        return false;
    }

    // Pre-size the results vector to avoid repeated reallocations under lock.
    allSummaryCardsVector.reserve(allSummaryCardsVector.size() + urls_.size());
    total_     = urls_.size();
    completed_ = 0;

    // -- Fill the work queue --------------------------------------------------
    WorkQueue queue;
    {
        std::lock_guard<std::mutex> lk(queue.mtx);
        for (const auto& url : urls_)
            queue.items.push(url);
    }

    // -- Spawn the pool -------------------------------------------------------
    const std::size_t n = std::min(pool_size_, urls_.size());
    std::vector<pthread_t>       tids(n);
    std::vector<PoolWorkerArg>   args(n);

    for (std::size_t i = 0; i < n; ++i) {
        args[i] = { this, &queue };
        int err = pthread_create(&tids[i], nullptr, &CaliforniaDashboardAPI::poolWorker, &args[i]);
        if (err) {
            fprintf(stderr, "runFullURLFetch: pthread_create failed: %s\n", strerror(err));
            // Signal remaining workers to stop, then join what we launched.
            {
                std::lock_guard<std::mutex> lk(queue.mtx);
                queue.done = true;
            }
            queue.cv.notify_all();
            for (std::size_t j = 0; j < i; ++j)
                pthread_join(tids[j], nullptr);
            return false;
        }
    }

    // -- Signal done after all URLs are queued (they already are) -------------
    {
        std::lock_guard<std::mutex> lk(queue.mtx);
        queue.done = true;
    }
    queue.cv.notify_all();

    // -- Wait for all workers to finish ---------------------------------------
    for (std::size_t i = 0; i < n; ++i)
        pthread_join(tids[i], nullptr);

    return true;
}

// =============================================================================
// poolWorker  —  static thread entry point
// =============================================================================

void* CaliforniaDashboardAPI::poolWorker(void* raw)
{
    auto* a = static_cast<PoolWorkerArg*>(raw);
    WorkQueue& q = *a->queue;

    while (true) {
        std::string url;

        // -- Pop one URL from the queue (or exit if drained) -----------------
        {
            std::unique_lock<std::mutex> lk(q.mtx);

            // Wait until there is work OR the queue is done.
            q.cv.wait(lk, [&q] {
                return !q.items.empty() || q.done;
            });

            if (q.items.empty()) break; // done and nothing left

            url = std::move(q.items.front());
            q.items.pop();
        }
        // Lock released — fetch happens outside the critical section.

        // -- Rate limiter: block until a token is available before fetching --
        a->self->acquireToken();

        // -- Fetch -----------------------------------------------------------
        SummaryCard card;
        a->self->fetchSummaryCard(url, card);

        // -- Push result (only the vector push needs protection) -------------
        {
            std::lock_guard<std::mutex> lk(a->self->mutex_);
            a->self->allSummaryCardsVector.push_back(std::move(card));
        }

        // -- Progress bar (writes to stderr, overwrites same line) -----------
        {
            std::size_t done  = ++a->self->completed_;
            std::size_t total = a->self->total_;
            int pct           = static_cast<int>(done * 100 / total);
            int filled        = pct / 2; // 50-char wide bar

            fprintf(stderr, "\r  [");
            for (int i = 0; i < 50; ++i)
                fputc(i < filled ? '#' : '-', stderr);
            fprintf(stderr, "] %3d%%  %zu/%zu", pct, done, total);
            fflush(stderr);

            if (done == total)
                fputc('\n', stderr); // final newline when complete
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
// acquireToken  —  token bucket rate limiter
// =============================================================================

/**
 * Blocks the calling thread until a request token is available.
 * Tokens refill at max_requests_per_sec_ per second. This ensures the total
 * request rate across ALL workers never exceeds the configured cap.
 *
 * Example: max_requests_per_sec=4.0 means at most 4 fetches/sec globally,
 * regardless of how many worker threads are running.
 */
void CaliforniaDashboardAPI::acquireToken()
{
    std::unique_lock<std::mutex> lk(rate_mutex_);

    while (true) {
        // Refill tokens based on elapsed time since last refill.
        struct timespec now;
        clock_gettime(CLOCK_MONOTONIC, &now);

        double elapsed = (now.tv_sec  - last_refill_.tv_sec) +
                         (now.tv_nsec - last_refill_.tv_nsec) / 1e9;

        tokens_ += elapsed * max_requests_per_sec_;
        if (tokens_ > max_requests_per_sec_)
            tokens_ = max_requests_per_sec_; // cap at bucket size (1 second worth)
        last_refill_ = now;

        if (tokens_ >= 1.0) {
            tokens_ -= 1.0; // consume one token
            return;
        }

        // Calculate how long to sleep until the next token arrives.
        double wait_sec = (1.0 - tokens_) / max_requests_per_sec_;
        long wait_ns    = static_cast<long>(wait_sec * 1e9);
        struct timespec sleep_ts = { wait_ns / 1'000'000'000L, wait_ns % 1'000'000'000L };

        lk.unlock();
        nanosleep(&sleep_ts, nullptr);
        lk.lock();
    }
}

// =============================================================================
// fetchSummaryCard
// =============================================================================

// Retryable CURL error codes — transient failures worth retrying.
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

CURLcode CaliforniaDashboardAPI::fetchSummaryCard(const std::string& url,
                                                   SummaryCard&       card)
{
    static constexpr int    MAX_RETRIES   = 4;
    static constexpr long   BASE_DELAY_MS = 500;  // doubles each retry: 500, 1000, 2000, 4000ms

    CURL* curl = curl_easy_init();
    if (!curl) {
        fprintf(stderr, "CURL initialization failed\n");
        return CURLE_FAILED_INIT;
    }

    curl_easy_setopt(curl, CURLOPT_URL,            url.c_str());
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION,  &CaliforniaDashboardAPI::write_callback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA,      &card);
    curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 1L);
    curl_easy_setopt(curl, CURLOPT_TIMEOUT_MS,     timeout_ms_);

    CURLcode result = CURLE_OK;

    for (int attempt = 0; attempt <= MAX_RETRIES; ++attempt) {
        // Clear any partial data from a previous failed attempt.
        if (attempt > 0) {
            card.clear();
            long delay_ms = BASE_DELAY_MS * (1L << (attempt - 1)); // exponential backoff
            fprintf(stderr, "[RETRY %d/%d] Waiting %ldms before retrying: %s\n",
                    attempt, MAX_RETRIES, delay_ms, url.c_str());
            struct timespec ts = { delay_ms / 1000, (delay_ms % 1000) * 1'000'000L };
            nanosleep(&ts, nullptr);
        }

        result = curl_easy_perform(curl);

        if (result == CURLE_OK) break; // success — exit retry loop

        fprintf(stderr, "CURL Error (attempt %d/%d) [%s]: %s\n",
                attempt + 1, MAX_RETRIES + 1, url.c_str(), curl_easy_strerror(result));

        if (!isRetryable(result)) break; // non-transient error — don't retry
    }

    if (result != CURLE_OK) {
        curl_easy_cleanup(curl);
        return result;
    }

    // Validate HTTP status — skip parsing on 4xx/5xx
    long http_code = 0;
    curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &http_code);
    if (http_code < 200 || http_code >= 300) {
        fprintf(stderr, "HTTP Error [%ld] for URL: %s\n", http_code, url.c_str());
        curl_easy_cleanup(curl);
        return CURLE_HTTP_RETURNED_ERROR;
    }

    // Guard against empty response
    const std::string& raw = card.getRawData();
    if (raw.empty()) {
        fprintf(stderr, "Empty response for URL: %s\n", url.c_str());
        curl_easy_cleanup(curl);
        return CURLE_GOT_NOTHING;
    }

    // Sanity check — valid JSON starts with '{' or '['
    size_t first = raw.find_first_not_of(" \t\r\n");
    if (first == std::string::npos || (raw[first] != '{' && raw[first] != '[')) {
        fprintf(stderr, "Invalid JSON for URL: %s\nResponse preview: %.200s\n",
                url.c_str(), raw.c_str());
        curl_easy_cleanup(curl);
        return CURLE_GOT_NOTHING;
    }

    card.parseRawData();
    curl_easy_cleanup(curl);
    return result;
}
