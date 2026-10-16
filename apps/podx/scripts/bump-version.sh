#!/usr/bin/env bash
# Bump LITEPOD_VERSION in api/.env (semver) — the single source of truth for the
# version, shared by local dev (`just api` loads api/.env via dotenvy) and
# `just podx-build`/`release-dockerhub`. Usage: bump-version.sh major|minor|patch
set -euo pipefail

level="${1:-patch}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
env_file="${script_dir}/../../api/.env"

if [[ ! -f "${env_file}" ]]; then
  echo "missing ${env_file}" >&2
  exit 1
fi

cur="$(grep -E '^LITEPOD_VERSION=' "${env_file}" | head -n1 | cut -d= -f2-)"
[[ "${cur}" =~ ^v([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || {
  echo "LITEPOD_VERSION=${cur} is not vMAJOR.MINOR.PATCH" >&2
  exit 1
}
major="${BASH_REMATCH[1]}"
minor="${BASH_REMATCH[2]}"
patch="${BASH_REMATCH[3]}"

case "${level}" in
  major) major=$((major+1)); minor=0; patch=0 ;;
  minor) minor=$((minor+1)); patch=0 ;;
  patch) patch=$((patch+1)) ;;
  *) echo "usage: $0 major|minor|patch" >&2; exit 2 ;;
esac

new="v${major}.${minor}.${patch}"
sed -i.bak "s|^LITEPOD_VERSION=.*|LITEPOD_VERSION=${new}|" "${env_file}"
rm -f "${env_file}.bak"
echo "${cur} -> ${new}"
