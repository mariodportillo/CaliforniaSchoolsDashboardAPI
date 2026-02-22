#include "SchoolResolver.hh"
#include "CaliforniaDashboardAPI.hh"

int main()
{
    SchoolResolver resolver("../pubschls.db");

    auto [urls, metadata] = resolver.buildURLs({"2021", "2022", "2023", "2024"});

    CaliforniaDashboardAPI api;
    api.loadInURLs(urls);
    api.runFullURLFetch();

    enrichCardsWithMetadata(api.allSummaryCardsVector, metadata);

    return 0;
}
