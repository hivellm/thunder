#!/usr/bin/env python3
"""Check that no registry is silently lagging the release train (PKG-011).

Thunder's central promise is one version everywhere: a wire-affecting change
lands in every language at once. Twice now a registry fell behind and *nothing
noticed* — npm when its publish job was removed for an OTP requirement, NuGet
when its API key was rejected — and both were found late, by a person, from
outside. A consumer hit the consequence first: a 0.1.0 C# client treats a
zero-length frame as a parse failure where a 0.2.0 client treats it as the
keep-alive WIRE-024 defines, so two Thunder clients disagreed about the same
server.

## What it checks, and what it deliberately does not

The naive check — "every registry must match the repo" — is wrong, and worse
than nothing: between releases the repo is *supposed* to be ahead of every
registry, so it would fail on nearly every commit and be ignored within a
week. A check people ignore is a check that will not be believed when it
matters.

So there are two modes:

- **tag** (a release): every registry must match the tag. This is the moment
  the promise is actually being made.
- **drift** (any other run): the registries must agree *with each other*.
  The repo being ahead of all of them is normal and passes. One registry
  behind the others is the failure this exists to catch.

Go and PHP are checked separately: both publish from a VCS tag with no registry
to query, and both live in their own repository — so each is reported, never
silently skipped.

Usage:
    check_published_versions.py drift
    check_published_versions.py tag 0.2.0
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

#: crates.io rejects requests without one, and a rejected request parses as
#: "nothing published" if the caller is careless — which is exactly the false
#: negative this script exists to prevent.
USER_AGENT = "thunder-release-check (https://github.com/hivellm/thunder)"

TIMEOUT = 30


class CheckError(RuntimeError):
    """A registry could not be reached or understood."""


def _get_json(url: str) -> dict:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT) as response:
            return json.load(response)
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
        raise CheckError(f"{url}: {exc}") from exc


def _version_key(version: str) -> tuple:
    """Sort key that orders 0.10.0 after 0.9.0, unlike string order."""
    return tuple(int(part) for part in re.findall(r"\d+", version))


def latest_crates() -> str:
    data = _get_json("https://crates.io/api/v1/crates/thunder-rpc")
    versions = [v["num"] for v in data.get("versions", []) if not v.get("yanked")]
    if not versions:
        raise CheckError("crates.io returned no versions for thunder-rpc")
    return max(versions, key=_version_key)


def latest_npm() -> str:
    data = _get_json("https://registry.npmjs.org/@hivehub/thunder")
    versions = list(data.get("versions", {}))
    if not versions:
        raise CheckError("npm returned no versions for @hivehub/thunder")
    return max(versions, key=_version_key)


def latest_pypi() -> str:
    data = _get_json("https://pypi.org/pypi/hivellm-thunder/json")
    versions = list(data.get("releases", {}))
    if not versions:
        raise CheckError("PyPI returned no releases for hivellm-thunder")
    return max(versions, key=_version_key)


def latest_nuget() -> str:
    data = _get_json(
        "https://api.nuget.org/v3-flatcontainer/hivellm.thunder/index.json"
    )
    versions = data.get("versions", [])
    if not versions:
        raise CheckError("NuGet returned no versions for HiveLLM.Thunder")
    return max(versions, key=_version_key)


REGISTRIES = {
    "crates.io": latest_crates,
    "npm": latest_npm,
    "PyPI": latest_pypi,
    "NuGet": latest_nuget,
}


def repo_version() -> str:
    """The version the repository claims, from the Rust workspace manifest."""
    text = (ROOT / "rust" / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not match:
        raise CheckError("could not read the version from rust/Cargo.toml")
    return match.group(1)


#: Lanes that resolve from a VCS tag instead of a registry API. Both live in
#: their own repository, so each is a place the train can silently fall out of
#: step — reported rather than skipped.
TAG_LANES = {
    "Go": ("thunder-go", "https://github.com/hivellm/thunder-go", "`go get` resolves no release"),
    "PHP": ("thunder-php", "https://github.com/hivellm/thunder-php", "Composer resolves no release"),
}


def tag_lane_state(lane: str, expected: str | None) -> str:
    """Report a VCS-tag lane, which has no registry to query."""
    repo, url, consequence = TAG_LANES[lane]
    try:
        tags = subprocess.run(
            ["git", "ls-remote", "--tags", url],
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
            check=False,
        )
    except (subprocess.SubprocessError, OSError) as exc:
        return f"{lane}: could not list tags ({exc})"
    if tags.returncode != 0:
        return f"{lane}: could not list tags (git exit {tags.returncode})"
    names = re.findall(r"refs/tags/(v[^\s^]+)$", tags.stdout, re.MULTILINE)
    if not names:
        return f"{lane}: {repo} has no version tag yet — {consequence}"
    latest = max(names, key=_version_key)
    if expected and latest.lstrip("v") != expected:
        return f"{lane}: {repo} latest tag is {latest}, expected v{expected}"
    return f"{lane}: {repo} latest tag is {latest}"


def collect() -> tuple[dict[str, str], list[str]]:
    published: dict[str, str] = {}
    errors: list[str] = []
    for name, fetch in REGISTRIES.items():
        try:
            published[name] = fetch()
        except CheckError as exc:
            errors.append(str(exc))
    return published, errors


def decide(
    mode: str, published: dict[str, str], expected: str | None = None
) -> tuple[int, list[str]]:
    """The whole judgement, separated from the network so it can be tested.

    Returns ``(exit_code, messages)``. Kept pure on purpose: the interesting
    part of this check is *when it fires*, and that has to be exercisable
    offline — a network blip must never be mistaken for a lagging registry,
    and neither must a passing test.
    """
    if mode == "tag":
        if not expected:
            return 1, ["::error::tag mode needs the expected version"]
        lagging = {n: v for n, v in published.items() if v != expected}
        if lagging:
            return 1, [
                f"::error::{name} published {version} but this release is "
                f"{expected} — the one-version promise (PKG-011) is broken"
                for name, version in sorted(lagging.items())
            ]
        return 0, [f"ok: every registry is at {expected}"]

    # drift: registries must agree with each other. The repo being ahead of
    # all of them is the normal state between releases and is NOT a failure —
    # a check that fires on every unreleased commit gets ignored, and an
    # ignored check is worse than none.
    distinct = set(published.values())
    if len(distinct) > 1:
        newest = max(distinct, key=_version_key)
        return 1, [
            f"::error::{name} is at {version} while others are at {newest} — "
            f"a registry is lagging the release train"
            for name, version in sorted(published.items())
            if version != newest
        ]
    if not distinct:
        return 1, ["::error::no registry reported a version"]
    return 0, [f"ok: every registry agrees at {distinct.pop()}"]


def main(argv: list[str]) -> int:
    mode = argv[1] if len(argv) > 1 else "drift"
    expected = argv[2].lstrip("v") if len(argv) > 2 else None

    published, errors = collect()
    for name, version in sorted(published.items()):
        print(f"  {name:<10} {version}")
    print(f"  {'repo':<10} {repo_version()}")
    for lane in TAG_LANES:
        print(f"  {tag_lane_state(lane, expected)}")

    if errors:
        # A registry we could not reach is not a lagging registry. Fail — a
        # silent pass would defeat the point — but do not claim a version gap
        # that was never observed.
        for error in errors:
            print(f"::error::registry unreachable: {error}")
        return 1

    code, messages = decide(mode, published, expected)
    for message in messages:
        print(message)
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv))
