#ifndef WEAVE_HOST_H
#define WEAVE_HOST_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Trusted local embedding ABI, host format 1; does not replace the legacy Program ABI.
 * Each input range must remain readable until return. Return is owned UTF-8 JSON;
 * release exactly once with weave_native_free. Handles are opaque ASCII tokens.
 * No Number conversion of graph/artifact data. Retain complete compiler bytes.
 * These wrappers open native SQLite. Browser image persistence must fence every
 * requires_fence outcome; poisoned means discard/reopen/inspect, never replay.
 * open/install/state are privileged host configuration, never remote request verbs. */
char *weave_host_open(const uint8_t *, size_t, const uint8_t *, size_t);
char *weave_host_call(const uint8_t *, size_t, const uint8_t *, size_t);
char *weave_host_close(const uint8_t *, size_t);
char *weave_host_artifact_select(const uint8_t *, size_t, const uint8_t *, size_t);
char *weave_host_install_handler(const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t *, size_t);
char *weave_host_set_adapter_state(const uint8_t *, size_t, const uint8_t *, size_t);
void weave_native_free(char *);
#ifdef __cplusplus
}
#endif
#endif
