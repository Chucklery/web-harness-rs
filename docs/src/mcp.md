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

## Planned

- workspace instructions
- search
- structured patch
- process execution
- background jobs
- structured Git
- approval handling

Planned capabilities are not exposed until their security model and tests are in place.

