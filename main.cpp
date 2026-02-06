#include <cstdio>
#include <curl/curl.h>
#include <iostream>
#include <nlohmann/json.hpp>
#include <string>

using json = nlohmann::json;

struct summaryCards {
  std::string rawData;
  json formatedData; 
};

size_t write_callback(char *ptr, size_t size, size_t nmemb, void *userdata) {
  size_t bytes = size * nmemb;
  summaryCards& data = *static_cast<summaryCards*>(userdata);  // Dereference to get reference
  data.rawData.append(ptr, bytes);
  std::cout << "chunk size: " << bytes << " bytes" << std::endl;
  return bytes;
};

CURLcode grab_Result(std::string url, CURL *curl, summaryCards& data) {
  if (!curl) {
    fprintf(stderr, "HTTP request failed\n");
    return CURLE_FAILED_INIT;
  }
  curl_easy_setopt(curl, CURLOPT_URL, url.c_str());
  curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, write_callback);
  curl_easy_setopt(curl, CURLOPT_WRITEDATA, &data);
 
  CURLcode result = curl_easy_perform(curl);

  if (result != CURLE_OK) {
    fprintf(stderr, "Error: %s\n", curl_easy_strerror(result));
  }
  curl_easy_cleanup(curl);
  return result;
}

int main() {
  curl_global_init(CURL_GLOBAL_DEFAULT);

  CURL *curl = curl_easy_init();
  std::string url = "https://api.caschooldashboard.org/Reports/19649071995901/11/SummaryCards";
  
  summaryCards data; 
  CURLcode result = grab_Result(url, curl, data);
  data.formatedData = json::parse(data.rawData);
  std::cout << data.formatedData.dump(2) << "\n";

  curl_global_cleanup();
  return 0;
}
