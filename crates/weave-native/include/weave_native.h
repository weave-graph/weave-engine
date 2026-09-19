#ifndef WEAVE_NATIVE_H
#define WEAVE_NATIVE_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Trusted embedding host only. Inputs are UTF-8 JSON with explicit byte lengths.
 * Caller keeps every input allocation readable until the call returns.
 * Returned NUL-terminated JSON belongs to caller; free exactly once with this library.
 * Calls serialize internally. Handle IDs are process-local and never reused.
 * Keep authority configuration separate from user graph source or peer messages. */
char *weave_native_open(const uint8_t *path, size_t path_len, const uint8_t *host_json, size_t host_len);
char *weave_native_execute(uint64_t handle, const uint8_t *program_json, size_t program_len);
char *weave_native_close(uint64_t handle);
void weave_native_free(char *response);
#ifdef __cplusplus
}
#endif
#endif
