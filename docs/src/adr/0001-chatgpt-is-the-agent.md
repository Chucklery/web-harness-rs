# ADR 0001: ChatGPT is the agent

Status: accepted.

ChatGPT owns reasoning, planning, conversation, and tool orchestration. web-harness provides local execution capabilities only.

This avoids nested agents, duplicate context, duplicate model calls, and unnecessary latency.

