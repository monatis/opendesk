"""Background scheduler daemon — runs scheduled tasks using APScheduler."""

from __future__ import annotations

import asyncio
import logging
import signal
import sys
import time
from pathlib import Path

logger = logging.getLogger(__name__)


def start_daemon(project_dir: Path) -> None:
    """Start the scheduler daemon (blocking). Ctrl-C to stop."""
    try:
        from apscheduler.schedulers.blocking import BlockingScheduler
        from apscheduler.triggers.cron import CronTrigger
        from apscheduler.triggers.interval import IntervalTrigger
    except ImportError:
        print(
            "ERROR: apscheduler is required for the scheduler.\n"
            "Install with: pip install 'opendesk[schedule]'",
            file=sys.stderr,
        )
        sys.exit(1)

    from opendesk.automation.schedule_store import ScheduleStore
    from opendesk.automation.runner import run_task

    store = ScheduleStore(project_dir)
    scheduler = BlockingScheduler()

    entries = store.all()
    if not entries:
        print(f"No schedules found in {project_dir}. Add one with the learn tool or schedule tool.")
        print("Waiting for schedules... (Ctrl-C to stop)")

    def _make_job(entry_name: str, task: str):
        def job():
            logger.info(f"Running scheduled task: {entry_name}")
            status = asyncio.run(run_task(task, project_dir))
            store.update_run(entry_name, status)
            logger.info(f"Task '{entry_name}' completed: {status}")
            print(f"[{_now()}] {entry_name}: {status}")
        return job

    for entry in entries:
        if not entry.enabled:
            continue
        try:
            if entry.cron:
                parts = entry.cron.split()
                trigger = CronTrigger(
                    minute=parts[0], hour=parts[1],
                    day=parts[2], month=parts[3], day_of_week=parts[4],
                )
            elif entry.interval_seconds:
                trigger = IntervalTrigger(seconds=entry.interval_seconds)
            else:
                logger.warning(f"Skipping '{entry.name}': no valid trigger")
                continue

            scheduler.add_job(
                _make_job(entry.name, entry.task),
                trigger=trigger,
                id=entry.id,
                name=entry.name,
                misfire_grace_time=60,
            )
            print(f"  Scheduled: {entry.name!r} — {entry.timing!r} → {entry.task!r}")
        except Exception as e:
            logger.error(f"Failed to schedule '{entry.name}': {e}")

    print(f"\nopendesk scheduler running. Ctrl-C to stop.\n")

    def _handle_sigterm(*_):
        scheduler.shutdown(wait=False)
        sys.exit(0)

    signal.signal(signal.SIGTERM, _handle_sigterm)

    try:
        scheduler.start()
    except (KeyboardInterrupt, SystemExit):
        print("\nScheduler stopped.")


def _now() -> str:
    import datetime
    return datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S")
