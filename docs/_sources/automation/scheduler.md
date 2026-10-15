# Scheduler

!!! warning
    In the Rust version, schedules are **stored only**. Nothing executes them:
    `opendesk scheduler start` just logs enabled schedules once a minute. Run
    the task yourself (or ask the agent to) when it matters.

## The `schedule` tool

| Action | Parameters | Description |
|--------|-----------|-------------|
| `add` | `name`, `task`, `timing` | Store a schedule (replaces one with the same name) |
| `remove` | `name` | Remove a schedule by name |
| `list` | — | Show stored schedules |

There is no `run` action; calling it returns an error.

## Ask Claude

> "Store a schedule called daily-backup: every day at 09:00, task: back up the reports folder"

> "List my scheduled tasks"

> "Remove the hourly-check schedule"

---

## Timing strings

`timing` is stored as free text and is not parsed or validated. The tool
description suggests forms like `every 30m`, `every 2h`,
`every day at 09:00`, `every friday at 17:00`.

---

## CLI

```bash
opendesk scheduler list  [--dir PROJECT_DIR]   # print stored schedules
opendesk scheduler start [--dir PROJECT_DIR]   # log enabled schedules every 60 s; runs nothing
```

`--dir` defaults to the current directory.

---

## Storage

| What | Where |
|---|---|
| Schedules | `<project>/.opendesk/schedules.json`, where `<project>` is the working directory of the MCP server (for the tool) or `--dir` / the current directory (for the CLI) |
| Procedures (`learn`) | `~/.opendesk/procedures/<name>.json` |

Both are plain JSON. A schedule entry has `id`, `name`, `task`, `timing`,
`enabled`, `created_at`, and the unused fields `cron`, `interval_seconds`,
`last_run`, `last_status`.

---

If you want to run tasks on a remote machine, check out [Remote Computer Use →](../remote/index.md)
