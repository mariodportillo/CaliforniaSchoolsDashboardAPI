#include "CaliforniaDashboardAPI.hh"

// Constructors and Destructor
CaliforniaDashboardAPI::CaliforniaDashboardAPI(long timeout_ms)
    : timeout_ms_(timeout_ms){
    CURLcode rc = curl_global_init(CURL_GLOBAL_ALL);
    if (rc != CURLE_OK)
        throw std::runtime_error(std::string("curl_global_init: ") +
                                 curl_easy_strerror(rc));
}

CaliforniaDashboardAPI::~CaliforniaDashboardAPI(){
    curl_global_cleanup();
}

bool CaliforniaDashboardAPI::loadInURLs(const std::vector<std::string> &urls)
{
    if (urls.empty()) {
        fprintf(stderr, "loadInURLs: provided URL list is empty\n");
        return false;
    }

    std::vector<std::string> valid;
    valid.reserve(urls.size());

    for (const auto &url : urls) {
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

bool CaliforniaDashboardAPI::runFullURLFetch()
{
    if (urls_.empty()) {
        fprintf(stderr, "runFullURLFetch: no valid URLs found in list, must load them in first.\n");
        return false;
    }

    const std::size_t n = urls_.size();
    std::vector<ThreadArg> args(n);
    std::vector<pthread_t> tids(n);

    // Spawn one thread per URL
    for (std::size_t i = 0; i < n; ++i) {
        args[i].self = this;
        args[i].url  = urls_[i];

        int err = pthread_create(&tids[i], nullptr, &CaliforniaDashboardAPI::threadWorker, &args[i]);
        if (err) {
            fprintf(stderr, "runFullURLFetch: pthread_create failed for %s: %s\n",
                    urls_[i].c_str(), strerror(err));
            // Join any threads already launched before returning
            for (std::size_t j = 0; j < i; ++j)
                pthread_join(tids[j], nullptr);
            return false;
        }
    }

    // Block until every thread has finished
    for (std::size_t i = 0; i < n; ++i)
        pthread_join(tids[i], nullptr);

    return true;
}

// ── static thread entry point ─────────────────────────────────────────────────

void *CaliforniaDashboardAPI::threadWorker(void *raw)
{
    auto *a = static_cast<ThreadArg *>(raw);

    // Each thread owns its own SummaryCard – no sharing during the fetch
    SummaryCard card;
    a->self->fetchSummaryCard(a->url, card);

    // Only the push into the shared vector needs protection
    a->self->mutex_.lock();
    a->self->allSummaryCardsVector.push_back(std::move(card));
    a->self->mutex_.unlock();

    return nullptr;
}

// ── write callback ────────────────────────────────────────────────────────────

size_t CaliforniaDashboardAPI::write_callback(char *ptr, size_t size,
                                               size_t nmemb, void *userdata)
{
    size_t bytes = size * nmemb;
    SummaryCard *card = static_cast<SummaryCard *>(userdata);
    card->appendRawData(ptr, bytes);
    return bytes;
}

// ── single URL fetch ──────────────────────────────────────────────────────────

CURLcode CaliforniaDashboardAPI::fetchSummaryCard(const std::string &url,
                                                   SummaryCard       &card)
{
    CURL *curl = curl_easy_init();
    if (!curl) {
        fprintf(stderr, "CURL initialization failed\n");
        return CURLE_FAILED_INIT;
    }

    curl_easy_setopt(curl, CURLOPT_URL,            url.c_str());
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION,  &CaliforniaDashboardAPI::write_callback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA,      &card);
    curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 1L);

    CURLcode result = curl_easy_perform(curl);
    if (result != CURLE_OK)
        fprintf(stderr, "CURL Error [%s]: %s\n", url.c_str(), curl_easy_strerror(result));

    card.parseRawData();
    curl_easy_cleanup(curl);
    return result;
}
