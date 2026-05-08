# Learn & Replay

The `learn` tool lets you record any desktop workflow once and replay it on demand. The agent watches your mouse, keyboard, and screen — then summarizes the recording into a reusable procedure.

---

## How it works

```
You perform a task  →  opendesk records every click, key, and screenshot
                    →  Agent summarizes into a procedure JSON
                    →  Saved to .opendesk/learned/<task-name>.json
                    →  Replay anytime: agent re-executes using ui/mouse/keyboard tools
```

Procedures are stored locally in your project directory. They describe **goals and steps**, not specific file paths or app versions — so they work across different machines and environments.

---

## In Claude Code (recommended)

No code needed. Just talk to Claude:

**Recording:**
> "Start recording task expense-report"

Perform the task yourself. Then:
> "Stop recording"

Claude captures the trajectory, summarizes it, and saves the procedure automatically.

**Replaying:**
> "Replay expense-report"

Claude loads the saved procedure and executes each step using the available tools.

**Listing saved tasks:**
> "List my learned tasks"

---

## Via Python

### Start recording

```python
from opendesk import create_registry, allow_all_context

registry = create_registry()
ctx = allow_all_context()
learn = registry.get("learn")

result = await learn.execute(ctx, learn.Params(
    action="start",
    task_name="expense-report"
))
print(result.output)
# Recording started for task 'expense-report'. Perform the task now...
```

Perform the workflow manually while the recording is active.

### Stop and review

```python
result = await learn.execute(ctx, learn.Params(action="stop"))
print(result.output)
# Shows full event log: clicks, keystrokes, timing, accessibility context
# Plus key screenshots from the session
```

### Save the procedure

After stopping, pass the trajectory to an LLM to summarize, then save:

```python
# The procedure JSON should follow this structure:
procedure = {
    "task_name": "expense-report",
    "description": "Fill and submit the monthly expense report form",
    "steps": [
        "Open the expense report application",
        "Click New Report",
        "Fill in date, amount, and category fields",
        "Attach receipt if required",
        "Click Submit"
    ],
    "procedure": "Navigate to the expense report tool. Create a new report entry. ..."
}

import json
result = await learn.execute(ctx, learn.Params(
    action="save",
    task_name="expense-report",
    procedure=json.dumps(procedure)
))
print(result.output)
# Procedure 'expense-report' saved to .opendesk/learned/expense-report.json
```

### Replay

```python
result = await learn.execute(ctx, learn.Params(
    action="replay",
    task_name="expense-report"
))
print(result.output)
# Returns step-by-step instructions for the agent to execute
```

### List all procedures

```python
result = await learn.execute(ctx, learn.Params(action="list"))
print(result.output)
# Learned procedures:
#   expense-report — Fill and submit the monthly expense report form
#   browser-search — Search for a term and open the first result
```

---

## Installation

Task recording requires `pynput` for global input capture:

```bash
pip install 'opendesk[core,mcp,learn]'
```

---

## Platform support

| Feature | macOS | Linux | Windows |
|---------|-------|-------|---------|
| Mouse recording | ✓ | ✓ | ✓ |
| Keyboard recording | ✓ | ✓ | ✓ |
| Screenshot capture | ✓ | ✓ | ✓ |
| Accessibility context | atomacos (optional) | pyatspi (optional) | pywinauto (optional) |

Accessibility context enriches recordings with element names and roles (e.g. `Button "Submit"` instead of just coordinates). It's optional — recordings work without it, but replay is more reliable with it.

**Install accessibility support:**

```bash
# macOS
pip install atomacos

# Linux
pip install pyatspi

# Windows
pip install pywinauto
```

---

## Where procedures are stored

```
your-project/
└── .opendesk/
    └── learned/
        ├── expense-report.json
        ├── browser-search.json
        └── form-filling.json
```

Each file is plain JSON — you can edit, share, or version-control them.

---

## How replay works

When you call `replay`, the agent receives a prompt containing:
- The task goal and description
- Step-by-step instructions written in plain language
- A reminder to use `ui` first, `screenshot(marks=True)` if needed, and `mouse` as a last resort

The agent then executes the steps using the current screen state — it does not replay pixel coordinates or hardcoded paths. This makes procedures portable across different machines, screen resolutions, and app versions.

---

## Tips

- **Name tasks clearly** — `expense-report` is better than `task1`. The name is used for fuzzy matching on replay.
- **Keep recordings focused** — one task per recording. Shorter recordings produce better procedures.
- **Record on a clean screen** — close unrelated windows before starting.
- **Procedures are generalized** — the agent writes steps like "click the Submit button", not "click at pixel (450, 320)". This makes them reusable.
