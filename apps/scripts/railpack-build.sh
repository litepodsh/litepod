#!/bin/sh
# Runs in the isolated, non-root Railpack build container. The API transfers
# only the prepared build inputs into this container; it never mounts /data.
set -eu

: "${IMAGE_TAG:?IMAGE_TAG is required}"
: "${CONTEXT_DIR:?CONTEXT_DIR is required}"
: "${PLAN_DIR:?PLAN_DIR is required}"
: "${OUT_DIR:?OUT_DIR is required}"

RAILPACK_FRONTEND="${RAILPACK_FRONTEND:-ghcr.io/railwayapp/railpack-frontend}"
BUILDKIT_ADDR="unix:///tmp/buildkit-runtime/buildkitd.sock"
BUILDKIT_LOG="$OUT_DIR/buildkitd.log"

# A rootless daemon per build keeps BuildKit's workers in this temporary
# container's cgroup. Podman has already placed this container in a rootless
# user namespace, so BuildKit runs as that mapped root rather than host root.
# The API applies the application's CPU/RAM limits to that cgroup. The native
# snapshotter avoids granting /dev/fuse to the build.
export XDG_RUNTIME_DIR=/tmp/buildkit-runtime
mkdir -p "$XDG_RUNTIME_DIR"
buildkitd --rootless --addr "$BUILDKIT_ADDR" \
    --root /tmp/buildkit \
    --oci-worker-snapshotter=native >"$BUILDKIT_LOG" 2>&1 &
BUILDKIT_PID=$!
cleanup_buildkit() {
    kill "$BUILDKIT_PID" 2>/dev/null || true
    wait "$BUILDKIT_PID" 2>/dev/null || true
}
trap cleanup_buildkit EXIT INT TERM

while [ ! -S /tmp/buildkit-runtime/buildkitd.sock ]; do
    if ! kill -0 "$BUILDKIT_PID" 2>/dev/null; then
        cat "$BUILDKIT_LOG" >&2 || true
        exit 1
    fi
    sleep 0.1
done

# Build-time environment. The API writes one KEY=VALUE per line so values never
# appear in argv (visible to anything that can read the process table).
ENV_FILE="$PLAN_DIR/build-env"
KEYS=""
if [ -f "$ENV_FILE" ]; then
    KEYS=$(cut -d= -f1 < "$ENV_FILE" | grep -v '^$' || true)
fi
if [ -n "$KEYS" ]; then
    while IFS= read -r line || [ -n "$line" ]; do
        [ -z "$line" ] && continue
        export "$line"
    done < "$ENV_FILE"
fi

# Railpack compiles those variables into BuildKit secret references, so every
# one of them has to be handed to buildctl as a secret or the build aborts with
# "secret NAME: not found". Only the names go through argv; buildctl reads the
# values from the environment exported above.
set --
if [ "${RAILPACK_CLEAN_BUILD:-true}" = "true" ]; then
    set -- "$@" --no-cache
fi
for key in $KEYS; do
    set -- "$@" --secret "id=$key,env=$key"
done

echo "==> Building image $IMAGE_TAG"
buildctl --addr "$BUILDKIT_ADDR" build \
    --progress plain \
    --local "context=$CONTEXT_DIR" \
    --local "dockerfile=$PLAN_DIR" \
    --frontend=gateway.v0 \
    --opt "source=$RAILPACK_FRONTEND" \
    --opt filename=railpack-plan.json \
    "$@" \
    --output "type=docker,name=$IMAGE_TAG,dest=$OUT_DIR/image.tar"

echo "==> Build finished"
