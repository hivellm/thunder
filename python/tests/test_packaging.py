"""Packaging invariants (PEP 561).

The annotations in ``thunder_rpc`` only reach a consumer if the distribution
carries a ``py.typed`` marker. Without it every downstream type checker treats
the package as untyped and silently degrades ``Value``, ``Client`` and
``Config`` to ``Any`` — at the wire boundary, which is where a protocol library
most wants to be strict (GH #7).

That is an easy thing to lose again: deleting one empty file, or changing the
build config so it stops being packaged, breaks it with no other symptom. So it
is pinned here.
"""

from __future__ import annotations

from pathlib import Path

import thunder_rpc


def test_package_ships_the_pep561_marker() -> None:
    """``py.typed`` sits beside the modules it vouches for."""
    package_dir = Path(thunder_rpc.__file__).parent
    marker = package_dir / "py.typed"
    assert marker.is_file(), (
        "thunder_rpc/py.typed is missing — without it PEP 561 says this "
        "package is untyped, and every consumer's checker ignores the "
        "annotations it already has"
    )


def test_the_marker_is_declared_as_package_data() -> None:
    """The build config keeps the marker in the built artifacts.

    Verifying the file exists in the source tree is not enough: it also has to
    survive into the wheel and the sdist. Hatchling includes package data
    automatically for the packages it is told about, so this asserts the
    package is declared — the link between "the file is here" and "the file
    ships".
    """
    pyproject = (Path(__file__).resolve().parents[1] / "pyproject.toml").read_text(
        encoding="utf-8"
    )
    assert 'packages = ["thunder_rpc"]' in pyproject, (
        "the wheel target must declare the thunder_rpc package, or py.typed "
        "stops being packaged"
    )
