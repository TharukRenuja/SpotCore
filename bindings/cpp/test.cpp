#include "spotcore.hpp"
#include <cassert>
#include <utility>
int main() {
    spotcore::session original;
    assert(original.operations().find("library.add") != std::string::npos);
    assert(original.invoke("library.contains", R"({"values":["spotify:track:test"]})").find("not_authenticated") != std::string::npos);
    assert(original.account().find("not_authenticated") != std::string::npos);
    spotcore::session moved = std::move(original);
    bool rejected = false;
    try { original.account(); } catch (const std::logic_error &) { rejected = true; }
    assert(rejected);
    assert(moved.poll_event().find("not_authenticated") != std::string::npos);
    assert(moved.shutdown().find("\"ok\":true") != std::string::npos);
    assert(moved.account().find("closed") != std::string::npos);
    moved.close();
    moved.close();
    rejected = false;
    try { moved.account(); } catch (const std::logic_error &) { rejected = true; }
    assert(rejected);
}
