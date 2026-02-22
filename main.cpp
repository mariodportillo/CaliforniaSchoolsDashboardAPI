#include "SchoolResolver.hh"
#include "CaliforniaDashboardAPI.hh"

int main()
{
    SchoolResolver resolver("../pubschls.db");

    SchoolResolver::BuildResult result   = resolver.buildURLs({"2021", "2022", "2023", "2024"});
    std::vector<std::string>&   urls     = result.urls;
    SchoolResolver::URLMetadata& metadata = result.metadata;

    CaliforniaDashboardAPI api;
    api.loadInURLs(urls);
    api.runFullURLFetch();

    enrichCardsWithMetadata(api.allSummaryCardsVector, metadata);

    return 0;
}
