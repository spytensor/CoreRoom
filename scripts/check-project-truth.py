#!/usr/bin/env python3
"""Fail fast when CoreRoom's high-signal project facts drift.

This intentionally checks only active, user-facing truth surfaces. Historical
release notes and architecture documents may mention old v0.x milestones.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def fail(message: str) -> None:
    print(f"FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def match(pattern: str, text: str, message: str) -> re.Match[str]:
    found = re.search(pattern, text, re.MULTILINE)
    if not found:
        fail(message)
    return found


def main() -> int:
    cargo = read("Cargo.toml")
    cargo_version = match(
        r'^version\s*=\s*"([^"]+)"',
        cargo,
        "Cargo.toml package version not found",
    ).group(1)

    npm_version = json.loads(read("npm/package.json"))["version"]
    require(
        cargo_version == npm_version,
        f"Cargo.toml version {cargo_version} != npm/package.json version {npm_version}",
    )

    changelog = read("CHANGELOG.md")
    released = [
        v
        for v in re.findall(r"^## \[([^\]]+)\]", changelog, re.MULTILINE)
        if v != "Unreleased"
    ]
    require(released, "CHANGELOG.md has no released version entries")
    require(
        released[0] == cargo_version,
        f"top CHANGELOG release {released[0]} != Cargo.toml version {cargo_version}",
    )

    readme = read("README.md")
    readme_tag = match(
        r"^TAG=v([0-9]+\.[0-9]+\.[0-9]+(?:[-+.][A-Za-z0-9.-]+)?)$",
        readme,
        "README direct-install TAG not found",
    ).group(1)
    require(
        readme_tag == cargo_version,
        f"README direct-install TAG v{readme_tag} != Cargo.toml version {cargo_version}",
    )

    agents = read("AGENTS.md")
    pr_template = read(".github/PULL_REQUEST_TEMPLATE.md")
    require(
        "Active milestone: v0.8" not in agents,
        "AGENTS.md still names v0.8 as active",
    )
    require(
        "Primary tracker: #238" not in agents,
        "AGENTS.md still names #238 as primary tracker",
    )
    require(
        "Do not pick up v0.9+ work while #238" not in agents,
        "AGENTS.md still blocks v0.9+ on #238",
    )
    require(
        "current active tracker is #238" not in pr_template,
        "PR template hard-codes #238 as active tracker",
    )

    bug_template = read(".github/ISSUE_TEMPLATE/bug.yml")
    require(
        ".coderoom/messages.jsonl" not in bug_template,
        "bug template still points at .coderoom/messages.jsonl",
    )
    require(
        ".coreroom/messages.jsonl" in bug_template,
        "bug template does not mention .coreroom/messages.jsonl",
    )

    gitignore = read(".gitignore")
    require(
        "/.coreroom/" in gitignore,
        ".gitignore does not ignore source-repo .coreroom/ dogfood state",
    )
    require(
        "/.coderoom/" in gitignore,
        ".gitignore dropped legacy .coderoom/ ignore",
    )

    print(
        f"project truth ok: v{cargo_version}; "
        "active tracker guidance is not hard-coded to #238"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
