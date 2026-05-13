"""opendesk app — the local browser UI for ``opendesk app``.

A single FastAPI process that:

* wraps a running :class:`~opendesk.remote.server.OpendeskServer` so this
  machine can be controlled by paired peers;
* maintains a cache of outbound :class:`~opendesk.computer.RemoteComputer`
  connections so the operator can control paired hosts from the UI;
* serves a vanilla-JS single-page UI from ``static/``.

The UI is intentionally not a framework — it's a small set of HTML/CSS/JS
files that talk to the REST endpoints in :mod:`opendesk.app.app`.
"""

from opendesk.app.app import AppState, create_app, run

__all__ = ["AppState", "create_app", "run"]
