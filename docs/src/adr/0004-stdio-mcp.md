# ADR 0004: Use stdio MCP locally

Status: accepted.

The default local MCP transport is stdio.

This avoids a listening socket, local HTTP authentication, CORS and CSRF concerns, and an additional server lifecycle.

