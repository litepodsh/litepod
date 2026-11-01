#!/usr/bin/env python3
"""Build the release manifest for the version being published.

Reads the current manifest on stdin, writes the updated one to stdout. Reading
the old one first preserves any keys added later (notes, links) instead of
flattening the document to whatever this script happens to know about. An
Only an empty object supplied by the publisher for a verified 404 represents
the first release. Invalid JSON and non-object documents are rejected so a
corrupt successful response can never be replaced as though it were absent.

Env: VERSION, CHANNEL (stable or alpha), CADDY_TAG, DRAGONFLY_TAG.
"""

import datetime
import json
import os
import sys


TAG_CHARS = set("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._-")
DIGEST_CHARS = set("0123456789abcdefABCDEF")


def core_tag(name: str) -> str:
    tag = os.environ.get(name, "").strip()
    if (
        tag == "latest"
        or not tag
        or len(tag) > 128
        or not tag[0].isalnum()
        or any(char not in TAG_CHARS for char in tag)
    ):
        raise ValueError(f"{name} must be a non-latest container tag")
    return tag


def app_digest() -> str | None:
    """The immutable image digest to pin this release to, or None when unset.

    Optional: an unsigned release still falls back to the mutable tag. When
    provided it must be `sha256:` + 64 hex characters, the form the registry
    reports as `Digest:`.
    """
    digest = os.environ.get("DIGEST", "").strip()
    if not digest:
        return None
    if len(digest) != len("sha256:") + 64 or not digest.startswith("sha256:"):
        raise ValueError("DIGEST must be sha256: followed by 64 hex characters")
    if any(char not in DIGEST_CHARS for char in digest[len("sha256:"):]):
        raise ValueError("DIGEST must be sha256: followed by 64 hex characters")
    return digest


def main() -> int:
    version = os.environ.get("VERSION", "").strip()
    if not version:
        print("VERSION is required", file=sys.stderr)
        return 1
    channel = os.environ.get("CHANNEL", "").strip()
    if channel not in {"stable", "alpha"}:
        print("CHANNEL must be stable or alpha", file=sys.stderr)
        return 1
    try:
        caddy_tag = core_tag("CADDY_TAG")
        dragonfly_tag = core_tag("DRAGONFLY_TAG")
        digest = app_digest()
    except ValueError as error:
        print(error, file=sys.stderr)
        return 1

    raw = sys.stdin.read()
    try:
        doc = json.loads(raw)
    except json.JSONDecodeError as error:
        print(f"current manifest is not valid JSON: {error}", file=sys.stderr)
        return 1
    if not isinstance(doc, dict):
        print("current manifest must be a JSON object", file=sys.stderr)
        return 1

    timestamp = datetime.datetime.now(datetime.timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:%SZ"
    )
    channels = doc.setdefault("channels", {})
    if not isinstance(channels, dict):
        channels = {}
        doc["channels"] = channels
    entry = channels.get(channel)
    if not isinstance(entry, dict):
        entry = {}
        channels[channel] = entry

    doc["schema"] = 1
    entry["version"] = version
    entry["released_at"] = timestamp
    if digest is not None:
        entry["digest"] = digest
    entry["caddy"] = {"tag": caddy_tag, "released_at": timestamp}
    entry["dragonfly"] = {"tag": dragonfly_tag, "released_at": timestamp}
    if channel == "stable":
        doc["version"] = version
        doc["released_at"] = timestamp

    json.dump(doc, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
