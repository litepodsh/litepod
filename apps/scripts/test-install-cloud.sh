#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=install-cloud.sh
source "${script_dir}/install-cloud.sh"

assert_fails() {
	if "$@"; then
		printf 'expected command to fail: %q\n' "$*" >&2
		exit 1
	fi
}

cloud_env="$(mktemp)"
standard_env="$(mktemp)"
trap 'rm -f -- "${cloud_env}" "${standard_env}"' EXIT

printf '%s\n' 'IS_CLOUD=true' > "${cloud_env}"
assert_cloud_installation "${cloud_env}"

printf '%s\n' 'LITEPOD_IMAGE_TAG=v1.0.0' > "${standard_env}"
assert_fails assert_cloud_installation "${standard_env}"

assert_cloud_installation "${TMPDIR:-/tmp}/litepod-cloud-install-does-not-exist"

assert_eq() {
	local expected="$1" actual="$2"
	if [[ "${expected}" != "${actual}" ]]; then
		printf 'expected %q, got %q\n' "${expected}" "${actual}" >&2
		exit 1
	fi
}

assert_eq v0.3.8 "${FALLBACK_LITEPOD_IMAGE_TAG}"
assert_eq 2.11.4 "${FALLBACK_CADDY_IMAGE_TAG}"
assert_eq v2.0.2 "${FALLBACK_DRAGONFLY_IMAGE_TAG}"

litepod_image_tag=''
caddy_image_tag=''
dragonfly_image_tag=''
use_fallback_image_tags
assert_eq "${FALLBACK_LITEPOD_IMAGE_TAG}" "${litepod_image_tag}"
assert_eq "${FALLBACK_CADDY_IMAGE_TAG}" "${caddy_image_tag}"
assert_eq "${FALLBACK_DRAGONFLY_IMAGE_TAG}" "${dragonfly_image_tag}"

rg -Fq 'IS_CLOUD=true' "${script_dir}/install-cloud.sh"
rg -Fq 'IS_CLOUD: "true"' "${script_dir}/install-cloud.sh"
rg -Fq 'RESEND_API_KEY=${resend_api_key}' "${script_dir}/install-cloud.sh"
rg -Fq 'RESEND_API_KEY: \${RESEND_API_KEY:?Set RESEND_API_KEY in .env}' "${script_dir}/install-cloud.sh"
rg -Fq 'RESEND_API_KEY: \${RESEND_API_KEY:-}' "${script_dir}/install.sh"
rg -Fq 'POLAR_WEBHOOK_SECRET=${POLAR_WEBHOOK_SECRET:-}' "${script_dir}/install-cloud.sh"
rg -Fq 'POLAR_WEBHOOK_SECRET: \${POLAR_WEBHOOK_SECRET:-}' "${script_dir}/install-cloud.sh"
rg -Fq 'use_fallback_image_tags' "${script_dir}/install-cloud.sh"
rg -Fq 'Version manifest is not valid JSON.' "${script_dir}/install-cloud.sh"
rg -Fq 'Version manifest returned an invalid ${requested_channel} tag' "${script_dir}/install-cloud.sh"
rg -Fq '\${CADDY_IMAGE_TAG:-${FALLBACK_CADDY_IMAGE_TAG}}' "${script_dir}/install-cloud.sh"
rg -Fq '\${DRAGONFLY_IMAGE_TAG:-${FALLBACK_DRAGONFLY_IMAGE_TAG}}' "${script_dir}/install-cloud.sh"
rg -Fq '\${LITEPOD_IMAGE_TAG:-${FALLBACK_LITEPOD_IMAGE_TAG}}' "${script_dir}/install-cloud.sh"

printf '%s\n' 'cloud installer tests passed'
