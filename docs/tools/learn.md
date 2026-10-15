# `learn` — Save and Replay Procedures

Stores written procedures so an agent can repeat a task later. The Rust
version **does not record** mouse or keyboard input: there is no `start` /
`stop` (calling them returns an error). The agent writes the procedure itself,
for example from the steps it just performed.

## Actions

| Action | Parameters | Description |
|--------|-----------|-------------|
| `save` | `task_name`, `procedure` | Save a procedure string (JSON) as `~/.opendesk/procedures/<task_name>.json` |
| `replay` | `task_name` | Return the saved procedure text for the agent to follow |
| `list` | — | List saved procedure names |

`replay` does not execute anything; it hands the stored steps back to the
agent, which carries them out with the normal tools against the current screen.

## Ask Claude

> "Save what you just did as a procedure called expense-report"

> "Replay the expense-report procedure"

> "What procedures have you saved?"

## Example calls

```json
{
  "action": "save",
  "task_name": "fill-form",
  "procedure": "{\"description\": \"Fill and submit the expense form\", \"steps\": [\"Open the form\", \"Fill in fields\", \"Click Submit\"]}"
}
{ "action": "replay", "task_name": "fill-form" }
{ "action": "list" }
```

The procedure is stored verbatim; no particular JSON shape is enforced.

See [Automation](../automation/index.md) for how this fits with `schedule`.
