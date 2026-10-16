#!/usr/bin/env python3
"""Executable regression tests for the release-manifest merge script."""

import json
import os
import subprocess
import sys
from pathlib import Path


SCRIPT = Path(__file__).with_name("merge-version-manifest.py")


def run_merge(current: dict, *, version: str, channel: str, caddy: str, dragonfly: str = "v1.39.0") -> dict:
    env = os.environ | {
        "VERSION": version,
        "CHANNEL": channel,
        "CADDY_TAG": caddy,
        "DRAGONFLY_TAG": dragonfly,
    }
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        input=json.dumps(current),
        text=True,
        capture_output=True,
        env=env,
        check=True,
    )
    return json.loads(result.stdout)


def test_stable_updates_root_mirror_without_touching_alpha() -> None:
    doc = run_merge(
        {
            "version": "v0.1.21",
            "channels": {"alpha": {"version": "v0.1.22-alpha.1"}},
        },
        version="v0.1.22",
        channel="stable",
        caddy="2.11.4",
    )
    assert doc["version"] == "v0.1.22"
    assert doc["channels"]["stable"]["version"] == "v0.1.22"
    assert doc["channels"]["stable"]["caddy"]["tag"] == "2.11.4"
    assert doc["channels"]["stable"]["dragonfly"]["tag"] == "v1.39.0"
    assert doc["channels"]["alpha"]["version"] == "v0.1.22-alpha.1"


def test_alpha_preserves_stable_and_root_mirror() -> None:
    doc = run_merge(
        {"version": "v0.1.22", "channels": {"stable": {"version": "v0.1.22", "caddy": {"tag": "2.11.4"}}}},
        version="v0.1.23-alpha.2",
        channel="alpha",
        caddy="2.12.0",
    )
    assert doc["version"] == "v0.1.22"
    assert doc["channels"]["stable"]["version"] == "v0.1.22"
    assert doc["channels"]["alpha"]["version"] == "v0.1.23-alpha.2"
    assert doc["channels"]["alpha"]["caddy"]["tag"] == "2.12.0"
    assert doc["channels"]["alpha"]["dragonfly"]["tag"] == "v1.39.0"


def test_merge_preserves_unknown_document_and_channel_fields() -> None:
    doc = run_merge(
        {
            "notes": "keep me",
            "channels": {"stable": {"url": "https://example.test/stable"}},
        },
        version="v0.1.22",
        channel="stable",
        caddy="2.11.4",
    )
    assert doc["notes"] == "keep me"
    assert doc["channels"]["stable"]["url"] == "https://example.test/stable"


def test_rejects_missing_or_invalid_channel() -> None:
    for channel in ("", "preview"):
        result = subprocess.run(
            [sys.executable, str(SCRIPT)],
            input="{}",
            text=True,
            capture_output=True,
            env=os.environ | {"VERSION": "v0.1.22", "CHANNEL": channel, "CADDY_TAG": "2.11.4", "DRAGONFLY_TAG": "v1.39.0"},
        )
        assert result.returncode == 1
        assert result.stderr == "CHANNEL must be stable or alpha\n"

def test_rejects_corrupt_or_non_object_current_manifest() -> None:
    for current in ("{broken", "[]", "null"):
        result = subprocess.run(
            [sys.executable, str(SCRIPT)],
            input=current,
            text=True,
            capture_output=True,
            env=os.environ | {"VERSION": "v0.1.22", "CHANNEL": "stable", "CADDY_TAG": "2.11.4", "DRAGONFLY_TAG": "v1.39.0"},
        )
        assert result.returncode == 1
        assert result.stderr.startswith("current manifest ")


def test_rejects_missing_or_latest_core_tags() -> None:
    for caddy in ["", "latest"]:
        result = subprocess.run(
            [sys.executable, str(SCRIPT)],
            input="{}",
            text=True,
            capture_output=True,
        env=os.environ | {"VERSION": "v0.1.22", "CHANNEL": "stable", "CADDY_TAG": caddy, "DRAGONFLY_TAG": "v1.39.0"},
        )
        assert result.returncode == 1


def main() -> int:
    tests = [
        test_stable_updates_root_mirror_without_touching_alpha,
        test_alpha_preserves_stable_and_root_mirror,
        test_merge_preserves_unknown_document_and_channel_fields,
        test_rejects_missing_or_invalid_channel,
        test_rejects_corrupt_or_non_object_current_manifest,
        test_rejects_missing_or_latest_core_tags,
    ]
    for test in tests:
        test()
        print(f"PASS {test.__name__}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
