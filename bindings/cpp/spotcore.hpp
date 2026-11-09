#ifndef SPOTCORE_HPP
#define SPOTCORE_HPP
#include "../c/spotcore.h"
#include <memory>
#include <mutex>
#include <stdexcept>
#include <string>
#include <string_view>

namespace spotcore {
inline std::string take_response(char *value) {
    std::unique_ptr<char, decltype(&spotcore_string_free)> result(value, &spotcore_string_free);
    if (!result) throw std::runtime_error("SpotCore returned no response");
    return result.get();
}
// Responses remain JSON so apps can keep their existing JSON library.
class session {
    struct state {
        std::mutex mutex;
        spotcore_session *handle;
        state() {
            char *error = nullptr;
            handle = spotcore_session_new(&error);
            if (!handle) throw std::runtime_error(error ? take_response(error) : "Session creation failed");
        }
        ~state() { spotcore_session_free(handle); }
    };
    std::unique_ptr<state> state_ = std::make_unique<state>();
public:
    session() = default;
    session(const session &) = delete;
    session &operator=(const session &) = delete;
    session(session &&) noexcept = default;
    session &operator=(session &&) noexcept = default;
    std::string call(std::string_view json) {
        if (!state_) throw std::logic_error("Session was moved from");
        std::lock_guard<std::mutex> lock(state_->mutex);
        if (!state_->handle) throw std::logic_error("Session is closed");
        return take_response(spotcore_session_call(state_->handle,
            reinterpret_cast<const uint8_t *>(json.data()), json.size()));
    }
    std::string operations() { return call(R"({"op":"operations"})"); }
    std::string invoke(std::string_view operation, std::string_view arguments = "{}") {
        if (operation.empty() || operation.find_first_not_of("abcdefghijklmnopqrstuvwxyz0123456789_.") != std::string_view::npos)
            throw std::invalid_argument("Invalid operation name");
        return call("{\"op\":\"invoke\",\"operation\":\"" + std::string(operation) + "\",\"arguments\":" + std::string(arguments) + "}");
    }
    std::string account() { return call(R"({"op":"account"})"); }
    std::string poll_event() { return call(R"({"op":"poll_event"})"); }
    std::string pause() { return call(R"({"op":"pause"})"); }
    std::string shutdown() { return call(R"({"op":"shutdown"})"); }
    void close() {
        if (!state_) return;
        std::lock_guard<std::mutex> lock(state_->mutex);
        spotcore_session_free(state_->handle);
        state_->handle = nullptr;
    }
};
}
#endif
