#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Handle one `metrics` frame (NUL-terminated JSON snapshot): update the model. */
void metrics_io_handle(uint8_t *payload, size_t len);

#ifdef __cplusplus
}
#endif
