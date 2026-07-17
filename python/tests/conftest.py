"""Test-suite plumbing: guarantees the tests directory is importable
(``mockserver``) and the package resolves from a source checkout."""

from __future__ import annotations

import sys
from pathlib import Path

_HERE = Path(__file__).resolve().parent
for path in (str(_HERE), str(_HERE.parent)):
    if path not in sys.path:
        sys.path.insert(0, path)
