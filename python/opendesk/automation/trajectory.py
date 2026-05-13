"""Trajectory data model for learn recordings."""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Optional


@dataclass
class TrajectoryEvent:
    timestamp: float
    action_type: str  # "click", "right_click", "scroll", "type", "key"
    x: Optional[int] = None
    y: Optional[int] = None
    button: Optional[str] = None   # "left", "right", "middle"
    text: Optional[str] = None     # for "type" events
    key: Optional[str] = None      # for "key" / modifier-combo events
    scroll_dx: int = 0
    scroll_dy: int = 0
    screenshot_after: Optional[str] = None  # base64 JPEG, set async after event
    # Semantic context from the OS accessibility API
    accessibility_context: Optional[dict] = None


@dataclass
class Trajectory:
    task_name: str
    events: list = field(default_factory=list)
    initial_screenshot: Optional[str] = None
    started_at: float = field(default_factory=time.time)
    stopped_at: float = 0.0


# ---------------------------------------------------------------------------
# Content builder  (multimodal — text + image blocks)
# ---------------------------------------------------------------------------

_MAX_SCREENSHOTS = 6
_SNAP_TYPES = frozenset({"click", "right_click"})
_SNAP_KEYS = frozenset({"return", "enter", "escape"})
_LEARN_PREFIXES = ("/learn ", "/learn\n", "/learn\r")


def _is_learn_command(ev: TrajectoryEvent) -> bool:
    if ev.action_type == "type" and ev.text:
        t = ev.text.strip().lower()
        return t.startswith("/learn")
    return False


def build_summarization_content(trajectory: Trajectory, procedure_path: str) -> list:
    """Return a list of content blocks (text + image_url) for a multimodal LLM call."""
    blocks: list = []

    def _text(t: str) -> dict:
        return {"type": "text", "text": t}

    def _image(b64: str) -> dict:
        return {
            "type": "image_url",
            "image_url": {"url": f"data:image/jpeg;base64,{b64}"},
        }

    blocks.append(
        _text(
            f"I recorded a computer task called '{trajectory.task_name}'.\n\n"
            "Please:\n"
            "1. Analyse the screenshots and action sequence below.\n"
            "2. Understand the **goal and intent** of the task.\n"
            f"3. Return ONLY a JSON object (no markdown fences, no explanation) with this structure:\n"
            "{\n"
            f'  "task_name": "{trajectory.task_name}",\n'
            '  "description": "one-line summary of what was accomplished",\n'
            '  "steps": ["step 1", "step 2", "..."],\n'
            '  "procedure": "..."\n'
            "}\n\n"
            "Guidelines for the procedure field:\n"
            "- Describe **what to accomplish at each step**, not which specific application, "
            "file format, filename, or directory path was used.\n"
            "- Never include absolute file paths, directory names, or filenames in the procedure.\n"
            "- Describe UI elements by their label or role, not pixel coordinates.\n"
            "- Write steps as goals an agent can re-execute in any equivalent environment.\n"
            "- Ignore any commands starting with '/learn' — those are recording control commands.\n\n"
            "--- RECORDING ---"
        )
    )

    if trajectory.initial_screenshot:
        blocks.append(_text("Initial screen state:"))
        blocks.append(_image(trajectory.initial_screenshot))

    screenshot_count = 0
    step = 0
    for ev in trajectory.events:
        if _is_learn_command(ev):
            continue
        step += 1
        elapsed = ev.timestamp - trajectory.started_at
        blocks.append(_text(f"Step {step} (+{elapsed:.1f}s): {_describe(ev)}"))

        want = (
            ev.screenshot_after is not None
            and screenshot_count < _MAX_SCREENSHOTS
            and (
                ev.action_type in _SNAP_TYPES
                or (ev.action_type == "key" and ev.key in _SNAP_KEYS)
            )
        )
        if want:
            blocks.append(_image(ev.screenshot_after))
            screenshot_count += 1

    blocks.append(_text("--- END OF RECORDING ---\n\nReturn only the JSON object."))
    return blocks


def build_display_summary(trajectory: Trajectory) -> str:
    """Return a clean, human-readable summary of the trajectory."""
    duration = (trajectory.stopped_at or time.time()) - trajectory.started_at
    visible_events = [e for e in trajectory.events if not _is_learn_command(e)]
    n_clicks = sum(1 for e in visible_events if e.action_type in _SNAP_TYPES)
    n_keys = sum(1 for e in visible_events if e.action_type in ("key", "type"))
    n_ss = sum(1 for e in visible_events if e.screenshot_after is not None)

    lines = [
        f"Task: {trajectory.task_name}",
        f"Duration: {duration:.1f}s  |  "
        f"{len(visible_events)} events  |  "
        f"{n_clicks} clicks  |  {n_keys} keyboard events  |  "
        f"{n_ss} screenshots captured",
        "",
        "Event log:",
    ]
    for i, ev in enumerate(visible_events):
        elapsed = ev.timestamp - trajectory.started_at
        lines.append(f"  {i + 1:2d}. (+{elapsed:5.1f}s) {_describe(ev)}")

    return "\n".join(lines)


def build_ax_replay_context(trajectory: Trajectory) -> dict:
    """Extract structured accessibility context from recorded events."""
    seen_apps: dict = {}
    events: list = []

    for ev in trajectory.events:
        if _is_learn_command(ev):
            continue
        ax = ev.accessibility_context or {}
        app = ax.get("app", "")
        window = ax.get("window", "")

        if app:
            seen_apps.setdefault(app, set()).add(window)

        entry: dict = {"action": ev.action_type}
        if app:
            entry["app"] = app
        if window:
            entry["window"] = window
        if ax.get("role"):
            entry["role"] = ax["role"]
        if ax.get("title"):
            entry["title"] = ax["title"]
        if ev.action_type == "key" and ev.key:
            entry["key"] = ev.key
        events.append(entry)

    apps = [
        {"name": name, "windows": sorted(wins - {""}) or []}
        for name, wins in seen_apps.items()
    ]
    return {"apps": apps, "events": events}


def _describe(ev: TrajectoryEvent) -> str:
    ax = ev.accessibility_context or {}
    app_s = f" [{ax['app']}]" if ax.get("app") else ""
    win_s = f" window='{ax['window']}'" if ax.get("window") else ""
    elem_s = ""
    if ax.get("role") or ax.get("title"):
        parts: list = []
        if ax.get("role"):
            parts.append(ax["role"])
        if ax.get("title"):
            parts.append(f'"{ax["title"]}"')
        elem_s = f" -> {' '.join(parts)}"

    if ev.action_type == "click":
        return f"Left-click at ({ev.x}, {ev.y}){app_s}{win_s}{elem_s}"
    if ev.action_type == "right_click":
        return f"Right-click at ({ev.x}, {ev.y}){app_s}{win_s}{elem_s}"
    if ev.action_type == "scroll":
        direction = "down" if ev.scroll_dy < 0 else "up"
        return f"Scroll {direction} at ({ev.x}, {ev.y}){app_s}{win_s}"
    if ev.action_type == "type":
        return f"Typed: {ev.text!r}{app_s}{win_s}{elem_s}"
    if ev.action_type == "key":
        return f"Key: {ev.key}{app_s}{win_s}"
    return f"Action: {ev.action_type}"
