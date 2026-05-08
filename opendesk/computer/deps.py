"""Lazy import helpers for optional computer-use dependencies."""

from __future__ import annotations

import importlib
from types import ModuleType


def ensure_import(package: str, install_hint: str | None = None) -> ModuleType:
    """Import *package* or raise a clear :class:`ImportError`.

    Parameters
    ----------
    package:
        The Python package name to import (e.g. ``"pyperclip"``).
    install_hint:
        Optional custom install instructions.  Defaults to
        ``pip install <package>``.
    """
    try:
        return importlib.import_module(package)
    except ImportError as exc:
        hint = install_hint or f"pip install {package}"
        raise ImportError(
            f"{package!r} is required but not installed.\n"
            f"Install it with:  {hint}"
        ) from exc
