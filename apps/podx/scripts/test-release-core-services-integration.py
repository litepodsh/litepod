#!/usr/bin/env python3
"""Regression checks for core-service publication in both release channels."""

import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
JUSTFILE = ROOT / "Justfile"
PUBLISH_VERSION = 'just publish-version'


def dry_run(*args: str) -> str:
    result = subprocess.run(
        ["just", "--dry-run", "release-dockerhub", *args],
        cwd=ROOT,
        text=True,
        capture_output=True,
    )
    assert result.returncode == 0, result.stderr
    # just writes dry-run recipes to stderr so command output remains clean.
    return result.stdout + result.stderr


def test_release_selects_channel_specific_core_pins() -> None:
    recipe = JUSTFILE.read_text()
    assert "CADDY_TAG=\"$(grep -E '^CADDY_VERSION=' api/.env" in recipe
    assert "CADDY_TAG=\"$(grep -E '^CADDY_VERSION_ALPHA=' api/.env" in recipe
    assert '"${tag}" != latest' in recipe


def test_stable_release_hands_its_pins_to_the_manifest_publisher() -> None:
    output = dry_run()
    assert "CADDY_TAG=\"$(grep -E '^CADDY_VERSION=' api/.env" in output
    assert PUBLISH_VERSION in output


def test_alpha_release_hands_its_pins_to_the_manifest_publisher() -> None:
    output = dry_run("--", "--alpha")
    assert "CADDY_TAG=\"$(grep -E '^CADDY_VERSION_ALPHA=' api/.env" in output
    assert PUBLISH_VERSION in output


def main() -> int:
    tests = [
        test_release_selects_channel_specific_core_pins,
        test_stable_release_hands_its_pins_to_the_manifest_publisher,
        test_alpha_release_hands_its_pins_to_the_manifest_publisher,
    ]
    for test in tests:
        test()
        print(f"PASS {test.__name__}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
