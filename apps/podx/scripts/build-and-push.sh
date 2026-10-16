#!/usr/bin/env bash
# Build the litepod image for linux/amd64 and push 3 tags to DockerHub.
# Source of truth: podx/.env
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
podx_dir="$(cd -- "${script_dir}/.." && pwd)"
apps_dir="$(cd -- "${podx_dir}/.." && pwd)"
env_file="${podx_dir}/.env"

if [[ ! -f "${env_file}" ]]; then
  echo "missing ${env_file} — copy from .env.example" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
. "${env_file}"
set +a

: "${DOCKERHUB_USER:?DOCKERHUB_USER required}"
: "${DOCKERHUB_REPO:?DOCKERHUB_REPO required}"
: "${DOCKERHUB_NAMESPACE:=docker.io}"
: "${VERSION:?VERSION required (run: just podx-bump patch)}"
: "${TARGET_PLATFORM:=linux/amd64}"

image_base="${DOCKERHUB_NAMESPACE}/${DOCKERHUB_USER}/${DOCKERHUB_REPO}"
tag_v="${VERSION}"
tag_channel="${CHANNEL:-qa}"
tag_latest="latest"

git_sha="$(git -C "${apps_dir}" rev-parse --short HEAD 2>/dev/null || echo unknown)"

echo ">> building ${image_base}:${tag_v} (${TARGET_PLATFORM})"
podman build \
  --platform="${TARGET_PLATFORM}" \
  -f "${apps_dir}/Dockerfile" \
  --build-arg VITE_API_URL="" \
  --build-arg LITEPOD_VERSION="${tag_v}" \
  --build-arg LITEPOD_GIT_SHA="${git_sha}" \
  -t "${image_base}:${tag_v}" \
  -t "${image_base}:${tag_channel}" \
  -t "${image_base}:${tag_latest}" \
  "${apps_dir}"

echo ">> pushing ${image_base}:${tag_v}"
podman push --platform="${TARGET_PLATFORM}" "${image_base}:${tag_v}"
echo ">> pushing ${image_base}:${tag_channel}"
podman push --platform="${TARGET_PLATFORM}" "${image_base}:${tag_channel}"
echo ">> pushing ${image_base}:${tag_latest}"
podman push --platform="${TARGET_PLATFORM}" "${image_base}:${tag_latest}"

echo ">> done — ${image_base}:${tag_v} | ${tag_channel} | ${tag_latest}"
