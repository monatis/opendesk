# Automation — Learn, Replay & Schedule

opendesk lets you record any desktop workflow, replay it on demand, and schedule it to run automatically. All three features work together as one system.

---

## How it works

```
Record a task once  →  opendesk captures clicks, keys, and screenshots
                    →  Agent summarizes into a reusable procedure JSON
                    →  Saved to .opendesk/learned/<name>.json
                    →  Replay anytime — agent re-executes using live tools
                    →  Schedule it — runs automatically on a timer
```

---

## In Claude Code

Just talk to Claude — no code needed.

**Record:**
> "Start recording task expense-report"

Perform the task yourself. Then:
> "Stop recording"

**Replay:**
> "Replay expense-report"

**Schedule:**
> "Schedule expense-report to run every friday at 5pm"
> "Schedule a task called hourly-check to take a screenshot every hour"

**Manage:**
> "List my learned tasks"
> "List my schedules"
> "Remove the hourly-check schedule"
> "Run the expense-report schedule now"

---

## Learn & Replay — Python API

```python
from opendesk import create_registry, allow_all_context

registry = create_registry()
ctx = allow_all_context()
learn = registry.get("learn")

# Start recording
await learn.execute(ctx, learn.Params(action="start", task_name="expense-report"))

# ... user performs the task ...

# Stop — returns event log + screenshots
result = await learn.execute(ctx, learn.Params(action="stop"))
print(result.output)

# Save the procedure (JSON summarized by an LLM)
import json
procedure = json.dumps({
    "task_name": "expense-report",
    "description": "Fill and submit the monthly expense report",
    "steps": ["Open the form", "Fill in all fields", "Click Submit"],
    "procedure": "Navigate to the expense report tool. Fill each required field. Submit."
})
await learn.execute(ctx, learn.Params(action="save", task_name="expense-report", procedure=procedure))

# Replay
result = await learn.execute(ctx, learn.Params(action="replay", task_name="expense-report"))
print(result.output)  # step-by-step instructions for the agent

# List all
result = await learn.execute(ctx, learn.Params(action="list"))
print(result.output)
```

---

## Schedule — Python API

```python
schedule = registry.get("schedule")

# Schedule a learned procedure
await schedule.execute(ctx, schedule.Params(
    action="add",
    name="expense-report",
    task="replay expense-report",
    timing="every friday at 17:00",
))

# Schedule a natural language task (no prior recording needed)
await schedule.execute(ctx, schedule.Params(
    action="add",
    name="hourly-screenshot",
    task="take a screenshot and save it to /tmp/screenshots/",
    timing="every 1h",
))

# List schedules
result = await schedule.execute(ctx, schedule.Params(action="list"))
print(result.output)

# Run immediately (for testing)
result = await schedule.execute(ctx, schedule.Params(action="run", name="expense-report"))

# Remove
await schedule.execute(ctx, schedule.Params(action="remove", name="hourly-screenshot"))
```

---

## Start the background runner

Schedules are saved to disk but only execute when the daemon is running:

```bash
opendesk scheduler start
```

To list schedules without starting the daemon:

```bash
opendesk scheduler list
```

Natural language tasks (non-replay) require Claude API access:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
export OPENDESK_MODEL=claude-sonnet-4-6  # optional, this is the default
opendesk scheduler start
```

---

## Timing formats

| Format | Example | Meaning |
|--------|---------|---------|
| Interval | `every 30m` | Every 30 minutes |
| Interval | `every 2h` | Every 2 hours |
| Interval | `every 1d` | Every day |
| Daily | `every day at 09:00` | Daily at 9am |
| Weekly | `every friday at 17:00` | Every Friday at 5pm |
| Weekly | `every monday at 8am` | Every Monday at 8am |
| Cron | `0 17 * * 5` | Raw cron expression |

---

## Installation

```bash
pip install 'opendesk[learn]'     # recording and replay (pynput)
pip install 'opendesk[schedule]'  # scheduled tasks (apscheduler)
pip install 'opendesk[all]'       # everything
```

---

## Storage

Procedures: `.opendesk/learned/<name>.json`
Schedules: `.opendesk/schedules.json`

Both are plain JSON — you can edit, share, or version-control them.

---

## Platform support

| Feature | macOS | Linux | Windows |
|---------|-------|-------|---------|
| Recording | ✓ | ✓ | ✓ |
| Accessibility context (optional) | atomacos | pyatspi | pywinauto |
| Replay | ✓ | ✓ | ✓ |
| Scheduling | ✓ | ✓ | ✓ |
