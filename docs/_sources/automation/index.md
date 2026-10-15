# Automation — Learn, Replay & Schedule

opendesk has two small storage tools for repeatable work. In the Rust version
both only **store** information; neither records your input nor runs tasks on
its own.

```
Agent performs a task with the normal tools
   →  learn(action=save) writes the steps as a procedure
      (~/.opendesk/procedures/<name>.json)
   →  learn(action=replay) returns those steps; the agent carries them out
   →  schedule(action=add) stores a name + task + timing
      (<project>/.opendesk/schedules.json) — nothing executes it
```

What is not available in the Rust version:

- **Recording.** `learn` cannot capture mouse / keyboard / screenshots;
  `start` and `stop` return an error.
- **Scheduled execution.** `opendesk scheduler start` only logs enabled
  schedules; it never runs their tasks. `schedule` has no `run` action.

## Ask Claude

> "Save what you just did as a procedure called expense-report"

> "Replay the expense-report procedure"

> "Store a schedule called weekly-report: every friday at 17:00"

> "List my schedules"

---

## Guides

- [In Claude Code](claude-code.md) — the natural-language requests that work
- [Scheduler](scheduler.md) — schedule storage, timing strings, the `scheduler` CLI
- [`learn` tool reference](../tools/learn.md)
