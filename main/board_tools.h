#pragma once

/* The tools the board offers the host (`tools/list`, `tools/call`), in the
 * shape of MCP tools so the host's MCP server can pass them through. */

#include "cJSON.h"

#ifdef __cplusplus
extern "C" {
#endif

/** `{"tools":[…]}` */
cJSON *board_tools_list(void);

/** Run tool `name`; NULL if there's no such tool. Otherwise a CallToolResult,
 * with `isError` set when the tool refused its arguments. */
cJSON *board_tools_call(const char *name, const cJSON *args);

#ifdef __cplusplus
}
#endif
