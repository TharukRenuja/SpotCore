#include "spotcore.h"
#include <assert.h>
#include <string.h>

static void check(spotcore_session *session, const char *request, const char *expected) {
    char *response = spotcore_session_call(session, (const uint8_t *)request, strlen(request));
    assert(response && strstr(response, expected));
    spotcore_string_free(response);
}
int main(void) {
    char *error = NULL;
    spotcore_session *session = spotcore_session_new(&error);
    assert(session && !error);
    check(session, "{\"op\":\"operations\"}", "library.add");
    check(session, "{\"op\":\"invoke\",\"operation\":\"library.contains\",\"arguments\":{\"values\":[\"spotify:track:test\"]}}", "not_authenticated");
    check(session, "{\"op\":\"account\"}", "not_authenticated");
    check(session, "{\"op\":\"poll_event\"}", "not_authenticated");
    check(session, "{\"op\":\"unknown\"}", "invalid_input");
    check(session, "{\"op\":\"next_event\"}", "unsupported");
    check(session, "{\"op\":\"shutdown\"}", "\"ok\":true");
    check(session, "{\"op\":\"account\"}", "closed");
    spotcore_session_free(session);
    spotcore_session_free(NULL);
    spotcore_string_free(NULL);
    return 0;
}
