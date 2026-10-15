# `audit` — Audit Log

Shows today's audit log of the machine running the MCP server. That log is
written by `opendesk serve` on a controlled machine: every protocol method a
remote controller called, with its outcome, plus session open / close /
rejected events (`~/.opendesk/audit/YYYY-MM-DD.jsonl`). Tool calls the MCP
server runs on the local machine are not recorded there.

## Ask Claude

> "Show me the audit log"

> "How many actions were recorded today?"

## Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `action` | `"show"` \| `"replay"` | `"show"` | Only `show` does anything; see below |
| `format` | `"summary"` \| `"full"` | `"full"` | `summary` = one-line count; `full` = up to 50 entries (method, summary, outcome) |

`session_id` and `skip_errors` are in the schema but are not used.

`action="replay"` does not replay anything over MCP; it returns a message
saying replay is not supported here.

## From the CLI

On the controlled machine:

```
opendesk audit [--date YYYY-MM-DD] [--peer NAME] [--limit N] [--follow]
```

---

Next: [learn →](learn.md)
