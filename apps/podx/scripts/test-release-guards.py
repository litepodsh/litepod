#!/usr/bin/env python3
"""Focused executable tests for release publication safety guards."""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


SCRIPT_DIR = Path(__file__).parent
PUBLISH = SCRIPT_DIR / "publish-version-manifest.sh"
IMMUTABLE = SCRIPT_DIR / "check-immutable-tag.sh"
WAIT_FOR_IMMUTABLE = SCRIPT_DIR / "wait-for-immutable-tag.sh"
UPLOAD = SCRIPT_DIR / "publish-install-scripts.sh"
VALIDATE_TAG = SCRIPT_DIR / "validate-release-tag.sh"
APP_ROOT = SCRIPT_DIR.parents[1]

FAKE_CURL = r"""#!/usr/bin/env python3
import json, os, pathlib, sys

args = sys.argv[1:]
state_path = pathlib.Path(os.environ["FAKE_STATE"])
state = json.loads(state_path.read_text()) if state_path.exists() else {"gets": 0, "puts": 0}
is_put = "-X" in args and args[args.index("-X") + 1] == "PUT"
url = args[-1]

def option(name):
    return args[args.index(name) + 1] if name in args else None

if is_put:
    index = state["puts"]
    statuses = os.environ.get("FAKE_PUT_STATUSES", "200").split(",")
    status = statuses[min(index, len(statuses) - 1)]
    state["puts"] += 1
    data_arg = option("--data-binary")
    if data_arg and data_arg.startswith("@"):
        state["last_put_body"] = pathlib.Path(data_arg[1:]).read_text()
    state.setdefault("put_args", []).append(args)
elif "auth.docker.io/token" in url:
    status = "200"
    state["auth_gets"] = state.get("auth_gets", 0) + 1
    out = option("-o")
    if out:
        pathlib.Path(out).write_text('{"token":"fake-registry-token"}')
elif "registry-1.docker.io/v2/" in url:
    index = state.get("registry_gets", 0)
    statuses = os.environ.get("FAKE_REGISTRY_STATUSES", "200").split(",")
    status = statuses[min(index, len(statuses) - 1)]
    state["registry_gets"] = index + 1
    bodies = json.loads(os.environ.get("FAKE_REGISTRY_BODIES", '["{\\"schemaVersion\\":2}"]'))
    out = option("-o")
    if out:
        pathlib.Path(out).write_text(bodies[min(index, len(bodies) - 1)])
else:
    index = state["gets"]
    statuses = os.environ.get("FAKE_GET_STATUSES", "200").split(",")
    status = statuses[min(index, len(statuses) - 1)]
    state["gets"] += 1
    bodies = json.loads(os.environ.get("FAKE_GET_BODIES", '["{}"]'))
    out = option("-o")
    if out:
        pathlib.Path(out).write_text(bodies[min(index, len(bodies) - 1)])
    headers = option("-D")
    if headers:
        pathlib.Path(headers).write_text(f'HTTP/1.1 {status}\r\nETag: "etag-{index}"\r\n\r\n')

state_path.write_text(json.dumps(state))
if int(status) >= 400 and any(arg.startswith("-") and "f" in arg[1:] for arg in args):
    raise SystemExit(22)
sys.stdout.write(status)
"""

def run(script: Path, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(script)],
        text=True,
        capture_output=True,
        env=os.environ | env,
        cwd=APP_ROOT,
    )


def fixture() -> tuple[tempfile.TemporaryDirectory, dict[str, str], Path]:
    scratch = tempfile.TemporaryDirectory()
    root = Path(scratch.name)
    curl = root / "curl"
    curl.write_text(FAKE_CURL)
    curl.chmod(0o755)
    state = root / "state.json"
    env = {
        "CURL_BIN": str(curl),
        "FAKE_STATE": str(state),
        "VERSION": "v1.2.3",
        "CHANNEL": "stable",
        "CADDY_TAG": "2.11.4",
        "DRAGONFLY_TAG": "v1.39.0",
        "URL": "https://objects.test/version.json",
        "S3_ACCESS_KEY": "key",
        "S3_SECRET_KEY": "secret",
        "IMAGE": "docker.io/example/litepod:v1.2.3",
        "REPOSITORY": "example/litepod",
        "TAG": "v1.2.3",
        "S3_ENDPOINT": "https://objects.test",
        "S3_BUCKET": "litepod",
        "S3_ACCESS_KEY": "key",
        "S3_SECRET_KEY": "secret",
    }
    return scratch, env, state


def test_publish_aborts_on_non_404_get_failure() -> None:
    scratch, env, state = fixture()
    with scratch:
        result = run(PUBLISH, env | {"FAKE_GET_STATUSES": "500"})
        assert result.returncode == 1
        assert json.loads(state.read_text())["puts"] == 0


def test_publish_uses_conditional_create_for_verified_404() -> None:
    scratch, env, state = fixture()
    with scratch:
        result = run(PUBLISH, env | {"FAKE_GET_STATUSES": "404"})
        assert result.returncode == 0, result.stderr
        calls = json.loads(state.read_text())
        assert "If-None-Match: *" in calls["put_args"][0]


def test_publish_retries_conflict_and_preserves_concurrent_channel() -> None:
    scratch, env, state = fixture()
    with scratch:
        bodies = [
            '{"channels":{"stable":{"version":"v1.2.2"}}}',
            '{"channels":{"stable":{"version":"v1.2.2"},"alpha":{"version":"v1.3.0-alpha.1"}}}',
        ]
        result = run(
            PUBLISH,
            env
            | {
                "FAKE_GET_STATUSES": "200,200",
                "FAKE_GET_BODIES": json.dumps(bodies),
                "FAKE_PUT_STATUSES": "412,200",
            },
        )
        assert result.returncode == 0, result.stderr
        calls = json.loads(state.read_text())
        assert calls["gets"] == 2 and calls["puts"] == 2
        assert "If-Match: \"etag-1\"" in calls["put_args"][1]
        final = json.loads(calls["last_put_body"])
        assert final["channels"]["alpha"]["version"] == "v1.3.0-alpha.1"


def test_immutable_tag_guard_reports_only_verified_registry_state() -> None:
    scratch, env, _ = fixture()
    with scratch:
        absent = run(IMMUTABLE, env | {"FAKE_REGISTRY_STATUSES": "404"})
        assert absent.returncode == 0 and absent.stdout.strip() == "absent"

        present = run(IMMUTABLE, env)
        assert present.returncode == 0 and present.stdout.strip() == "present"

        unavailable = run(IMMUTABLE, env | {"FAKE_REGISTRY_STATUSES": "500"})
        assert unavailable.returncode == 1
        assert "HTTP 500" in unavailable.stderr

        malformed = run(
            IMMUTABLE,
            env | {"FAKE_REGISTRY_BODIES": '["{}"]'},
        )
        assert malformed.returncode == 1
        assert "invalid manifest" in malformed.stderr


def test_immutable_tag_wait_retries_only_a_missing_registry_manifest() -> None:
    scratch, env, state = fixture()
    with scratch:
        result = run(
            WAIT_FOR_IMMUTABLE,
            env
            | {
                "FAKE_REGISTRY_STATUSES": "404,404,200",
                "MAX_ATTEMPTS": "3",
                "RETRY_DELAY_SECS": "0",
                "SLEEP_BIN": "true",
            },
        )
        assert result.returncode == 0, result.stderr
        assert json.loads(state.read_text())["registry_gets"] == 3

    scratch, env, state = fixture()
    with scratch:
        result = run(
            WAIT_FOR_IMMUTABLE,
            env
            | {
                "FAKE_REGISTRY_STATUSES": "404",
                "MAX_ATTEMPTS": "2",
                "RETRY_DELAY_SECS": "0",
                "SLEEP_BIN": "true",
            },
        )
        assert result.returncode == 1
        assert "after 2 attempts" in result.stderr
        assert json.loads(state.read_text())["registry_gets"] == 2


def test_release_recipe_waits_for_registry_before_announcing_release() -> None:
    recipe = (APP_ROOT / "justfile").read_text()
    immutable_push = 'podman manifest push --all "${local_manifest}" "${immutable_image}"'
    wait = 'bash podx/scripts/wait-for-immutable-tag.sh'
    moving_push = 'podman manifest push --all "${local_manifest}" "${moving_image}"'
    upload = 'bash podx/scripts/publish-install-scripts.sh'
    publish = 'just publish-version -- --alpha'

    assert recipe.index(immutable_push) < recipe.index(wait) < recipe.index(moving_push)
    assert recipe.index(wait) < recipe.index(upload) < recipe.index(publish)


def test_release_recipe_builds_and_checks_the_requested_version() -> None:
    recipe = (APP_ROOT / "justfile").read_text()
    build = 'podman build "${build_cache_args[@]}" --platform=linux/amd64'
    inspect = 'podman image inspect "$1" | python3 -c'
    check = 'if [[ "${built_version}" != "${VERSION}" ]]; then'
    immutable_check = 'immutable_state="$(IMAGE="${immutable_image}"'

    assert recipe.index(build) < recipe.index(inspect) < recipe.index(check) < recipe.index(immutable_check)


def test_release_recipe_uses_cache_unless_no_cache_is_requested() -> None:
    recipe = (APP_ROOT / "justfile").read_text()

    assert 'no_cache=false' in recipe
    assert '--no-cache)\n' in recipe
    assert 'build_cache_args+=(--no-cache)' in recipe
    assert 'podman rmi -f "${amd64_image}"' not in recipe
    assert 'podman rmi -f "${arm64_image}"' not in recipe


def test_release_workflow_uses_the_fixed_litepod_dockerhub_target() -> None:
    recipe = (APP_ROOT / "justfile").read_text()

    for recipe_name in ("podx-lint *ARGS:", "release-dockerhub *ARGS:", "publish-version *ARGS:"):
        start = recipe.index(recipe_name)
        end = recipe.find("\n# ", start + len(recipe_name))
        body = recipe[start:] if end == -1 else recipe[start:end]
        assert "DOCKERHUB_USER=litepod" in body
        assert "DOCKERHUB_REPO=litepod" in body

    assert 'immutable_image="docker.io/${DOCKERHUB_USER}/${DOCKERHUB_REPO}:${VERSION}"' in recipe
    assert 'moving_image="docker.io/${DOCKERHUB_USER}/${DOCKERHUB_REPO}:${moving_tag}"' in recipe
    assert 'tag_url="https://hub.docker.com/v2/repositories/${DOCKERHUB_USER}/${DOCKERHUB_REPO}/tags/${VERSION}"' in recipe


def test_release_recipe_verifies_compose_cli_before_registry_or_push() -> None:
    recipe = (APP_ROOT / "justfile").read_text()
    version_check = 'if [[ "${built_version}" != "${VERSION}" ]]; then'
    podman_check = 'podman run --rm --platform="linux/${a}" --entrypoint /bin/sh "${probe_image}" -c \'test -x /usr/bin/podman\''
    subcommand_check = 'dpkg --compare-versions "$(dpkg-query -W podman | cut -f2)" ge 4.7'
    compose_check = 'test -x /usr/bin/podman-compose && dpkg-query -W podman-compose'
    ca_bundle_check = 'podman run --rm --platform="linux/${a}" --entrypoint /usr/bin/test "${probe_image}" -s /etc/ssl/certs/ca-certificates.crt'
    immutable_check = 'immutable_state="$(IMAGE="${immutable_image}"'
    push = 'podman manifest push --all "${local_manifest}" "${immutable_image}"'

    assert recipe.index(version_check) < recipe.index(podman_check)
    assert recipe.index(podman_check) < recipe.index(subcommand_check)
    assert recipe.index(subcommand_check) < recipe.index(compose_check)
    assert recipe.index(compose_check) < recipe.index(ca_bundle_check)
    assert recipe.index(ca_bundle_check) < recipe.index(immutable_check)
    assert recipe.index(ca_bundle_check) < recipe.index(push)


def test_lifecycle_upload_publishes_all_scripts_and_fails_closed() -> None:
    scratch, env, state = fixture()
    with scratch:
        result = run(UPLOAD, env | {"FAKE_PUT_STATUSES": "200,200,200"})
        assert result.returncode == 0, result.stderr
        calls = json.loads(state.read_text())
        assert calls["puts"] == 3
        joined = "\n".join(" ".join(args) for args in calls["put_args"])
        assert "https://objects.test/litepod/install.sh" in joined
        assert "https://objects.test/litepod/install-cloud.sh" in joined
        assert "https://objects.test/litepod/uninstall.sh" in joined

    scratch, env, state = fixture()
    with scratch:
        result = run(UPLOAD, env | {"FAKE_PUT_STATUSES": "200,200,500"})
        assert result.returncode != 0
        assert json.loads(state.read_text())["puts"] == 3


def test_channel_tags_are_strictly_validated() -> None:
    valid = [
        ("stable", "v1.2.3"),
        ("alpha", "v1.2.3-alpha.4"),
    ]
    invalid = [
        ("stable", "latest"),
        ("stable", "v1.2.3-alpha.4"),
        ("alpha", "alpha"),
        ("alpha", "v1.2.3"),
        ("preview", "v1.2.3"),
    ]
    for channel, tag in valid:
        result = subprocess.run(["bash", str(VALIDATE_TAG), channel, tag], cwd=APP_ROOT)
        assert result.returncode == 0
    for channel, tag in invalid:
        result = subprocess.run(
            ["bash", str(VALIDATE_TAG), channel, tag],
            cwd=APP_ROOT,
            capture_output=True,
            text=True,
        )
        assert result.returncode == 1


def main() -> int:
    tests = [
        test_publish_aborts_on_non_404_get_failure,
        test_publish_uses_conditional_create_for_verified_404,
        test_publish_retries_conflict_and_preserves_concurrent_channel,
        test_immutable_tag_guard_reports_only_verified_registry_state,
        test_immutable_tag_wait_retries_only_a_missing_registry_manifest,
        test_release_recipe_waits_for_registry_before_announcing_release,
        test_release_recipe_builds_and_checks_the_requested_version,
        test_release_recipe_uses_cache_unless_no_cache_is_requested,
        test_release_workflow_uses_the_fixed_litepod_dockerhub_target,
        test_release_recipe_verifies_compose_cli_before_registry_or_push,
        test_lifecycle_upload_publishes_all_scripts_and_fails_closed,
        test_channel_tags_are_strictly_validated,
    ]
    for test in tests:
        test()
        print(f"PASS {test.__name__}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
