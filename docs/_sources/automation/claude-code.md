# Automation in Claude Code

Just talk to Claude — no code needed.

## Save a procedure

Ask Claude to do the task, then:

> "Save the steps you just took as a procedure called expense-report"

opendesk cannot record you doing the task yourself; Claude writes the
procedure from its own steps (or from your description).

## Replay

> "Replay expense-report"

Claude loads the saved steps and performs them again with the normal tools.

## Schedule

> "Store a schedule called expense-report: every friday at 17:00"

This only stores the schedule. Nothing runs it automatically in the Rust
version, and there is no "run the schedule now" action; ask Claude to perform
the task directly when you need it.

## Manage

> "List my saved procedures"

> "List my schedules"

> "Remove the hourly-check schedule"

---

Details: [Scheduler →](scheduler.md) · [`learn` reference →](../tools/learn.md)
