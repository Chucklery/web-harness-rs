# MCP Tool Model

The architecture prefers a small number of high-value tools instead of dozens of narrow tools.

## Implemented now

### workspace_info

Returns the canonical configured workspace root.

### read_files

Reads up to 16 UTF-8 files per call.

Current limits:

- 256 KiB per file
- 512 KiB total per batch
- all paths must canonicalize inside the workspace

### search

Uses the system ripgrep binary and returns at most 200 bounded matches.

### workspace_instructions

Discovers AGENTS.md files from the workspace root down to the target path and returns them in root-to-leaf order.

### patch

Applies bounded Codex-style Add File, Update File, and Delete File operations. Paths are workspace-scoped, updates require matching context, ambiguous hunks are rejected, and writes use a same-directory temporary file followed by rename.

## Planned

- process execution
- background jobs
- structured Git
- approval handling

Planned capabilities are not exposed until their security model and tests are in place.

