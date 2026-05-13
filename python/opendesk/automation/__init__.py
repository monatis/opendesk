"""Automation — record, replay, and schedule desktop tasks."""

from opendesk.automation.recorder import LearnRecorder
from opendesk.automation.storage import list_procedures, load_procedure, procedure_path, save_procedure
from opendesk.automation.schedule_store import ScheduleStore, ScheduleEntry
from opendesk.automation.runner import run_task

__all__ = [
    "LearnRecorder",
    "list_procedures",
    "load_procedure",
    "procedure_path",
    "save_procedure",
    "ScheduleStore",
    "ScheduleEntry",
    "run_task",
]
