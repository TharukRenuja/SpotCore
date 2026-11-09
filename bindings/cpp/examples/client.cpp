#include "../spotcore.hpp"
#include <iostream>
#include <string>

// Read newline-delimited JSON requests from stdin; never echo credential input.
int main() {
    spotcore::session spotify;
    for (std::string request; std::getline(std::cin, request);)
        std::cout << spotify.call(request) << '\n';
}
