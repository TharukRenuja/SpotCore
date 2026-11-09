#ifndef SPOTCORE_H
#define SPOTCORE_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

typedef struct spotcore_session spotcore_session;
#define SPOTCORE_MAX_REQUEST_BYTES (1024u * 1024u)

/* error_out may be NULL. Free any returned error with spotcore_string_free. */
spotcore_session *spotcore_session_new(char **error_out);
/* Blocking UTF-8 JSON call; response is owned by the caller. No input is retained.
   A handle must remain live for every call. Event reads use poll_event. */
char *spotcore_session_call(spotcore_session *session, const uint8_t *data, size_t length);
/* No calls may be in flight. Waits for shutdown; NULL is accepted. */
void spotcore_session_free(spotcore_session *session);
void spotcore_string_free(char *response);
#ifdef __cplusplus
}
#endif
#endif
