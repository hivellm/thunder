"""Offline tests for the release-train guard (GH #8).

The point of this check is *when it fires*, so that is what is tested — with
fixed inputs, no network. Two failure modes matter equally:

- missing a lagging registry (the bug that let NuGet sit a version behind
  while nobody noticed);
- firing on the normal state, which trains everyone to ignore it. An ignored
  check is worse than no check, because it looks like coverage.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from check_published_versions import _version_key, decide  # noqa: E402

ALIGNED = {
    "crates.io": "0.2.0",
    "npm": "0.2.0",
    "PyPI": "0.2.0",
    "NuGet": "0.2.0",
}
NUGET_BEHIND = {**ALIGNED, "NuGet": "0.1.0"}


def test_aligned_registries_pass_drift() -> None:
    code, messages = decide("drift", ALIGNED)
    assert code == 0, messages


def test_a_lagging_registry_fails_drift_and_is_named() -> None:
    """The exact situation from the issue: NuGet stuck while the rest moved."""
    code, messages = decide("drift", NUGET_BEHIND)
    assert code == 1
    joined = "\n".join(messages)
    assert "NuGet" in joined, joined
    assert "0.1.0" in joined and "0.2.0" in joined, joined


def test_repo_ahead_of_every_registry_is_not_a_failure() -> None:
    """The normal state between releases must stay green.

    Every commit after a release and before the next one has the repo ahead of
    all four registries. If that failed, the check would be red almost always
    and would be ignored by the time it mattered.
    """
    code, _ = decide("drift", ALIGNED)  # repo version is irrelevant in drift mode
    assert code == 0


def test_tag_mode_demands_every_registry_match_the_tag() -> None:
    code, _ = decide("tag", ALIGNED, "0.2.0")
    assert code == 0

    code, messages = decide("tag", ALIGNED, "0.3.0")
    assert code == 1
    assert all("0.3.0" in m for m in messages), messages


def test_tag_mode_names_only_the_lagging_registry() -> None:
    code, messages = decide("tag", NUGET_BEHIND, "0.2.0")
    assert code == 1
    joined = "\n".join(messages)
    assert "NuGet" in joined
    assert "npm" not in joined and "PyPI" not in joined, joined


def test_tag_mode_without_a_version_is_an_error_not_a_pass() -> None:
    code, _ = decide("tag", ALIGNED, None)
    assert code == 1


def test_no_registries_reported_is_a_failure() -> None:
    """An empty result must never read as agreement.

    This is the shape of the bug that made a failed crates.io request look
    like "nothing is published" — silence is not consensus.
    """
    code, _ = decide("drift", {})
    assert code == 1


def test_version_ordering_is_numeric_not_lexical() -> None:
    assert _version_key("0.10.0") > _version_key("0.9.0")
    assert max(["0.9.0", "0.10.0"], key=_version_key) == "0.10.0"
