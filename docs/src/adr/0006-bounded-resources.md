# ADR 0006: Bound resources and share permission policy

Status: accepted.

Model-facing reads, search results, output buffers, jobs, and future patch or execution inputs must have hard bounds.

All future mutation and execution paths should share one permission model rather than implementing independent ad-hoc checks.

