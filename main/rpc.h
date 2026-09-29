#pragma once

/* JSON-RPC 2.0 on the link's `ctrl` channel (docs/protocol.md). */

#include <stddef.h>
#include <stdint.h>

#include "cJSON.h"

#ifdef __cplusplus
extern "C" {
#endif

void rpc_init(void);

/** Handle one `ctrl` frame (NUL-terminated JSON) and send the response. */
void rpc_handle(uint8_t *payload, size_t len);

/** Send the `ready` notification: the board is up (again). */
void rpc_announce(void);

/** Send notification `method` with `params` (taken over; NULL for none). Any task. */
void rpc_notify(const char *method, cJSON *params);

#ifdef __cplusplus
}
#endif
