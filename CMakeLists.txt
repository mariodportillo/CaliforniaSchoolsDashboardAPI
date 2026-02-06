cmake_minimum_required(VERSION 3.10)
project(caDashboardAPI)
set(CMAKE_CXX_STANDARD 20)

file(GLOB SRC_FILES "*.cpp")
add_executable(main ${SRC_FILES})

find_package(CURL REQUIRED)
find_package(nlohmann_json 3.2.0 REQUIRED)

target_link_libraries(main PRIVATE CURL::libcurl nlohmann_json::nlohmann_json)
