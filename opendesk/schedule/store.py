"""Persistent storage for scheduled tasks."""

from __future__ import annotations

import json
import time
import uuid
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Optional


@dataclass
class ScheduleEntry:
    id: str
    name: str
    task: str                    # natural language task OR "replay <procedure-name>"
    timing: str                  # human-readable: "every 1h", "every friday 5pm", "0 17 * * 5"
    cron: Optional[str] = None   # parsed cron expression (None = interval-based)
    interval_seconds: Optional[int] = None  # for interval-based schedules
    enabled: bool = True
    created_at: float = field(default_factory=time.time)
    last_run: Optional[float] = None
    last_status: Optional[str] = None  # "ok" or error message


def schedules_path(project_dir: Path) -> Path:
    d = project_dir / ".opendesk"
    d.mkdir(parents=True, exist_ok=True)
    return d / "schedules.json"


class ScheduleStore:
    def __init__(self, project_dir: Path) -> None:
        self._path = schedules_path(project_dir)
        self._entries: list[ScheduleEntry] = self._load()

    def _load(self) -> list[ScheduleEntry]:
        if not self._path.exists():
            return []
        try:
            data = json.loads(self._path.read_text())
            return [ScheduleEntry(**e) for e in data]
        except Exception:
            return []

    def _save(self) -> None:
        self._path.write_text(
            json.dumps([asdict(e) for e in self._entries], indent=2)
        )

    def add(self, name: str, task: str, timing: str) -> ScheduleEntry:
        cron, interval_seconds = parse_timing(timing)
        entry = ScheduleEntry(
            id=str(uuid.uuid4())[:8],
            name=name,
            task=task,
            timing=timing,
            cron=cron,
            interval_seconds=interval_seconds,
        )
        # Replace existing entry with same name
        self._entries = [e for e in self._entries if e.name != name]
        self._entries.append(entry)
        self._save()
        return entry

    def remove(self, name: str) -> bool:
        before = len(self._entries)
        self._entries = [e for e in self._entries if e.name != name]
        if len(self._entries) < before:
            self._save()
            return True
        return False

    def get(self, name: str) -> Optional[ScheduleEntry]:
        for e in self._entries:
            if e.name == name:
                return e
        return None

    def all(self) -> list[ScheduleEntry]:
        return list(self._entries)

    def update_run(self, name: str, status: str) -> None:
        for e in self._entries:
            if e.name == name:
                e.last_run = time.time()
                e.last_status = status
                break
        self._save()


# ---------------------------------------------------------------------------
# Timing parser
# ---------------------------------------------------------------------------

_DAY_MAP = {
    "monday": "mon", "tuesday": "tue", "wednesday": "wed",
    "thursday": "thu", "friday": "fri", "saturday": "sat", "sunday": "sun",
}


def parse_timing(timing: str) -> tuple:
    """Parse a timing string into (cron_expr, interval_seconds).

    Supports:
      - Cron expressions:  "0 17 * * 5"
      - Interval:          "every 30m", "every 2h", "every 1d"
      - Daily:             "every day at 09:00"
      - Weekly:            "every friday at 17:00", "every friday 5pm"
    """
    t = timing.strip().lower()

    # Raw cron — 5 fields
    parts = t.split()
    if len(parts) == 5 and all(
        p.replace("*", "").replace("/", "").replace("-", "").replace(",", "").isdigit() or p == "*"
        for p in parts
    ):
        return timing.strip(), None

    # "every Xm / Xh / Xd"
    if t.startswith("every ") and len(parts) == 2:
        val = parts[1]
        if val.endswith("m") and val[:-1].isdigit():
            return None, int(val[:-1]) * 60
        if val.endswith("h") and val[:-1].isdigit():
            return None, int(val[:-1]) * 3600
        if val.endswith("d") and val[:-1].isdigit():
            return None, int(val[:-1]) * 86400

    # "every day at HH:MM"
    if t.startswith("every day"):
        hh, mm = _extract_time(t)
        return f"{mm} {hh} * * *", None

    # "every <weekday> at/@ HH:MM" or "every friday 5pm"
    for day_name, day_abbr in _DAY_MAP.items():
        if day_name in t:
            hh, mm = _extract_time(t)
            # cron: minute hour * * weekday
            from calendar import day_abbr as _abbrs
            day_num = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].index(day_abbr)
            return f"{mm} {hh} * * {day_num}", None

    # Fallback: treat as interval in seconds if purely numeric
    if t.isdigit():
        return None, int(t)

    raise ValueError(
        f"Cannot parse timing: {timing!r}\n"
        "Examples: 'every 30m', 'every 2h', 'every day at 09:00', "
        "'every friday at 17:00', '0 17 * * 5'"
    )


def _extract_time(text: str) -> tuple:
    """Extract (hour, minute) from a timing string."""
    import re
    # HH:MM
    m = re.search(r"(\d{1,2}):(\d{2})", text)
    if m:
        return int(m.group(1)), int(m.group(2))
    # 5pm / 9am
    m = re.search(r"(\d{1,2})\s*(am|pm)", text)
    if m:
        h = int(m.group(1))
        if m.group(2) == "pm" and h != 12:
            h += 12
        if m.group(2) == "am" and h == 12:
            h = 0
        return h, 0
    # Default: midnight
    return 0, 0
