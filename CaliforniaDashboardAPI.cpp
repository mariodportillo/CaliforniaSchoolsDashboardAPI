#include "CaliforniaDashboardAPI.hh"
#include <cstring>
#include <cstdio>
#include <stdexcept>
#include <time.h>
#include <unistd.h>
#include <netdb.h>
#include <arpa/inet.h>
#include <sys/socket.h>

// =============================================================================
// CURLSH lock/unlock callbacks — required for thread-safe share handle
// =============================================================================

// CURLSH needs external lock/unlock functions because it has no threading
// knowledge of its own. We use four mutexes (one per data type it shares).
static pthread_mutex_t share_locks[CURL_LOCK_DATA_LAST];

static void share_lock(CURL*, curl_lock_data data, curl_lock_access, void*) {
    pthread_mutex_lock(&share_locks[data]);
}
static void share_unlock(CURL*, curl_lock_data data, void*) {
    pthread_mutex_unlock(&share_locks[data]);
}

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

    // Detect CA bundle path once at construction — avoids races when
    // many threads all try to auto-detect it simultaneously at startup.
    const char* ca_candidates[] = {
        "/etc/ssl/cert.pem",                        // macOS Homebrew curl
        "/etc/ssl/certs/ca-certificates.crt",       // Debian/Ubuntu
        "/etc/pki/tls/certs/ca-bundle.crt",         // RHEL/CentOS
        "/usr/local/etc/openssl/cert.pem",           // macOS MacPorts
        nullptr
    };
    for (int i = 0; ca_candidates[i]; ++i) {
        if (access(ca_candidates[i], R_OK) == 0) {
            ca_bundle_path_ = ca_candidates[i];
            fprintf(stderr, "[SSL] Using CA bundle: %s\n", ca_bundle_path_.c_str());
            break;
        }
    }
    if (ca_bundle_path_.empty())
        fprintf(stderr, "[SSL] No CA bundle found — curl will use its default\n");

    // Initialise share-lock mutexes
    for (int i = 0; i < CURL_LOCK_DATA_LAST; ++i)
        pthread_mutex_init(&share_locks[i], nullptr);

    // Create the shared handle — workers share DNS cache and SSL sessions.
    // This means DNS is resolved once and the result is reused by all workers,
    // and TLS session tickets are shared so resumed handshakes are faster.
    curl_share_ = curl_share_init();
    if (curl_share_) {
        curl_share_setopt(curl_share_, CURLSHOPT_LOCKFUNC,   share_lock);
        curl_share_setopt(curl_share_, CURLSHOPT_UNLOCKFUNC, share_unlock);
        curl_share_setopt(curl_share_, CURLSHOPT_SHARE, CURL_LOCK_DATA_DNS);
        // SSL session sharing removed — causes CA cert corruption under high concurrency
    }
}

CaliforniaDashboardAPI::~CaliforniaDashboardAPI() {
    if (curl_share_) curl_share_cleanup(curl_share_);
    for (int i = 0; i < CURL_LOCK_DATA_LAST; ++i)
        pthread_mutex_destroy(&share_locks[i]);
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

    const std::size_t total    = urls_.size();
    const std::size_t base_slot = allSummaryCardsVector.size(); // existing elements
    total_     = total;
    completed_ = 0;
    next_slot_ = base_slot; // start slots AFTER any pre-existing cards

    // Pre-size the results vector to exactly the number of URLs.
    // Workers write directly into their pre-allocated slot using an atomic
    // index — no mutex required on the hot path at all.
    allSummaryCardsVector.resize(base_slot + total);

    // Fill work queue
    WorkQueue queue;
    {
        std::lock_guard<std::mutex> lk(queue.mtx);
        for (const auto& url : urls_)
            queue.items.push(url);
    }

    const std::size_t n = std::min(pool_size_, total);
    std::vector<pthread_t>     tids(n);
    std::vector<PoolWorkerArg> args(n);

    // -- Pre-resolve the API hostname ONCE before spawning any workers --------
    // All 50 workers firing DNS lookups simultaneously overwhelms the local
    // resolver. Instead we resolve once here, cache the result, and inject
    // it into every handle via CURLOPT_RESOLVE so workers skip DNS entirely.
    static const std::string API_HOST = "api.caschooldashboard.org";
    struct curl_slist* resolve_list = nullptr;
    {
        // Use getaddrinfo to resolve the hostname
        struct addrinfo hints{}, *res = nullptr;
        hints.ai_family   = AF_UNSPEC;
        hints.ai_socktype = SOCK_STREAM;
        if (getaddrinfo(API_HOST.c_str(), "443", &hints, &res) == 0 && res) {
            char ipbuf[INET6_ADDRSTRLEN] = {};
            if (res->ai_family == AF_INET) {
                inet_ntop(AF_INET,
                    &reinterpret_cast<sockaddr_in*>(res->ai_addr)->sin_addr,
                    ipbuf, sizeof(ipbuf));
            } else if (res->ai_family == AF_INET6) {
                inet_ntop(AF_INET6,
                    &reinterpret_cast<sockaddr_in6*>(res->ai_addr)->sin6_addr,
                    ipbuf, sizeof(ipbuf));
            }
            freeaddrinfo(res);

            if (ipbuf[0]) {
                // Format: "hostname:port:ip"
                std::string entry = API_HOST + ":443:" + ipbuf;
                std::string entry80 = API_HOST + ":80:" + ipbuf;
                resolve_list = curl_slist_append(resolve_list, entry.c_str());
                resolve_list = curl_slist_append(resolve_list, entry80.c_str());
                fprintf(stderr, "[DNS] Pre-resolved %s -> %s\n", API_HOST.c_str(), ipbuf);
            }
        } else {
            fprintf(stderr, "[DNS] Pre-resolve failed — workers will resolve individually\n");
        }
    }

    // Initialise one persistent CURL handle per worker
    for (std::size_t i = 0; i < n; ++i) {
        CURL* curl = curl_easy_init();
        if (!curl) {
            fprintf(stderr, "runFullURLFetch: curl_easy_init failed for worker %zu\n", i);
            for (std::size_t j = 0; j < i; ++j) curl_easy_cleanup(args[j].curl);
            return false;
        }

        // Attach the shared DNS cache
        if (curl_share_)
            curl_easy_setopt(curl, CURLOPT_SHARE, curl_share_);

        // Inject pre-resolved IP — workers never touch DNS again
        if (resolve_list)
            curl_easy_setopt(curl, CURLOPT_RESOLVE, resolve_list);

        // Use the CA bundle path detected once at construction
        if (!ca_bundle_path_.empty())
            curl_easy_setopt(curl, CURLOPT_CAINFO, ca_bundle_path_.c_str());
        curl_easy_setopt(curl, CURLOPT_SSL_VERIFYPEER, 1L);
        curl_easy_setopt(curl, CURLOPT_SSL_VERIFYHOST, 2L);

        // Browser identity — set once, inherited for all requests
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

        // TCP keep-alive so idle sockets don't get closed between requests
        curl_easy_setopt(curl, CURLOPT_TCP_KEEPALIVE, 1L);
        curl_easy_setopt(curl, CURLOPT_TCP_KEEPIDLE,  30L);
        curl_easy_setopt(curl, CURLOPT_TCP_KEEPINTVL, 15L);

        // Extended DNS cache TTL
        curl_easy_setopt(curl, CURLOPT_DNS_CACHE_TIMEOUT, 300L);

        // HTTP/2 — allows request multiplexing on a single TCP connection.
        // Falls back to HTTP/1.1 automatically if the server doesn't support it.
        curl_easy_setopt(curl, CURLOPT_HTTP_VERSION, CURL_HTTP_VERSION_2TLS);

        // Disable Nagle — reduces latency for small request/response cycles
        curl_easy_setopt(curl, CURLOPT_TCP_NODELAY, 1L);

        curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 1L);
        curl_easy_setopt(curl, CURLOPT_TIMEOUT_MS,     timeout_ms_);
        curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION,  &CaliforniaDashboardAPI::write_callback);

        args[i] = { this, &queue, curl, 0 };
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
            for (std::size_t j = 0; j < n;       ++j) curl_easy_cleanup(args[j].curl);
            return false;
        }
        ++spawned;
    }

    // Signal queue is fully loaded
    {
        std::lock_guard<std::mutex> lk(queue.mtx);
        queue.done = true;
    }
    queue.cv.notify_all();

    for (std::size_t i = 0; i < spawned; ++i)
        pthread_join(tids[i], nullptr);

    for (std::size_t i = 0; i < n; ++i)
        curl_easy_cleanup(args[i].curl);

    if (resolve_list)
        curl_slist_free_all(resolve_list);

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

        // Global rate limiter
        a->self->acquireToken();

        // Claim a slot in the pre-sized results vector — lock-free
        std::size_t slot = a->self->next_slot_.fetch_add(1, std::memory_order_relaxed);

        // Fetch directly into the pre-allocated slot — no lock needed
        a->self->fetchSummaryCard(a->curl, url, a->self->allSummaryCardsVector[slot]);

        // Progress bar — atomic increment first, then only lock stderr
        // every ~0.25% of total work to avoid the mutex becoming a bottleneck.
        {
            std::size_t done  = ++a->self->completed_;
            std::size_t total = a->self->total_;
            std::size_t print_every = std::max(std::size_t(1), total / 400);

            if (done % print_every == 0 || done == total) {
                std::lock_guard<std::mutex> lk(a->self->progress_mutex_);
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
// acquireToken  —  global token bucket
// =============================================================================

void CaliforniaDashboardAPI::acquireToken()
{
    // If unlimited, return immediately — zero overhead on the hot path.
    if (max_requests_per_sec_ >= 1000.0) return;

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

        if (tokens_ >= 1.0) { tokens_ -= 1.0; return; }

        double wait_sec = (1.0 - tokens_) / max_requests_per_sec_;
        long   wait_ns  = static_cast<long>(wait_sec * 1e9);
        struct timespec ts = { wait_ns / 1'000'000'000L, wait_ns % 1'000'000'000L };
        lk.unlock();
        nanosleep(&ts, nullptr);
        lk.lock();
    }
}

// =============================================================================
// fetchSummaryCard
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
    static constexpr int  MAX_RETRIES   = 3;
    static constexpr long BASE_DELAY_MS = 250; // shorter backoff at high speed

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
