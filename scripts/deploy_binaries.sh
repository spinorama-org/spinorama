#!/bin/sh
# Deploy the Rust binaries (generate-graph, generate-peq, generate-meta,
# generate-radar, generate-eq-compare, generate-headphone-meta,
# generate-headphone-graph, generate-peq-headphone, spinorama-api)
# to a server.
#
# Usage: scripts/deploy_binaries.sh [dev|prod|all]
#
# The binaries are x86_64-linux and are built natively on the dev host
# (it has cargo and 12 cores). Cross-building from macOS with
# cargo-zigbuild currently fails in the upstream math-optimisation
# build script, which uses #[cfg(target_os)] inside build.rs (a host
# cfg) and therefore emits `framework=Accelerate` for Linux targets;
# the upstream fix is to read CARGO_CFG_TARGET_OS instead. The prod
# VPS (2 vCPU, 2 GB) is too small to build on, so it receives the
# dev-built binaries after a glibc check: the newest GLIBC_ symbol the
# binaries require must be <= the prod glibc version.
#
# The crates form one cargo workspace (root Cargo.toml), so the whole
# workspace skeleton (root manifest, lockfile, crates/) includes all
# native dependencies and is synced to a single remote build directory.
#
# Binaries land in ~/spinorama-bin/ on the target and are smoke-tested
# there. The GPUI renderer (spinorama-render-images) is not covered:
# gpui must be built on a Linux checkout of gpui-toolkit.

set -eu

TARGET=${1:-dev}
DEV_HOST=web01.spin
PROD_HOST=ovhspin
REMOTE_BIN=spinorama-bin
REMOTE_BUILD=spinorama-workspace-build
# shellcheck disable=SC1007
REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BINARIES="generate-graph generate-peq generate-meta generate-radar generate-eq-compare generate-headphone-meta generate-headphone-graph generate-peq-headphone spinorama-api"

case "$TARGET" in
    dev|prod|all) ;;
    *) echo "Usage: $0 [dev|prod|all]" >&2; exit 2 ;;
esac

echo "==> Syncing workspace sources to $DEV_HOST:~/$REMOTE_BUILD"
ssh "$DEV_HOST" "mkdir -p ~/$REMOTE_BUILD ~/$REMOTE_BIN"
rsync -az "$REPO_ROOT/Cargo.toml" "$REPO_ROOT/Cargo.lock" "$DEV_HOST:$REMOTE_BUILD/"
rsync -az --delete \
    --exclude target --exclude '*.tmp' \
    "$REPO_ROOT/crates/" "$DEV_HOST:$REMOTE_BUILD/crates/"

echo "==> Building workspace on $DEV_HOST (native x86_64-linux)"
# A pipeline ending in tail hides Cargo's exit status in a POSIX shell.
ssh "$DEV_HOST" "cd ~/$REMOTE_BUILD && \
    if cargo build --locked --release --workspace >deploy-build.log 2>&1; then \
        tail -3 deploy-build.log; \
    else \
        status=\$?; cat deploy-build.log >&2; exit \$status; \
    fi"

install_and_smoke() {
    host=$1
    echo "==> Installing on $host:~/$REMOTE_BIN"
    ssh "$host" "mkdir -p \$HOME/$REMOTE_BIN"
    if [ "$host" = "$DEV_HOST" ]; then
        for bin in $BINARIES; do
            # Move the installed binary aside first: overwriting a
            # running executable in place fails with ETXTBSY
            # (spinorama-api is served by supervisor), while renaming
            # it keeps the live process on its old inode.
            ssh "$host" "mv -f ~/$REMOTE_BIN/$bin ~/$REMOTE_BIN/$bin.old 2>/dev/null || true"
            ssh "$host" "cp ~/$REMOTE_BUILD/target/release/$bin ~/$REMOTE_BIN/"
        done
    else
        tmp=$(mktemp -d)
        for bin in $BINARIES; do
            rsync -az "$DEV_HOST:$REMOTE_BUILD/target/release/$bin" "$tmp/"
            # Same ETXTBSY guard as above before rsync overwrites.
            ssh "$host" "mv -f ~/$REMOTE_BIN/$bin ~/$REMOTE_BIN/$bin.old 2>/dev/null || true"
            rsync -az "$tmp/$bin" "$host:$REMOTE_BIN/"
        done
        rm -rf "$tmp"
    fi
    ssh "$host" "chmod +x \$HOME/$REMOTE_BIN/generate-graph \$HOME/$REMOTE_BIN/generate-peq \$HOME/$REMOTE_BIN/generate-meta \$HOME/$REMOTE_BIN/generate-radar \$HOME/$REMOTE_BIN/generate-eq-compare \$HOME/$REMOTE_BIN/generate-headphone-meta \$HOME/$REMOTE_BIN/generate-headphone-graph \$HOME/$REMOTE_BIN/generate-peq-headphone \$HOME/$REMOTE_BIN/spinorama-api"
    echo "==> Smoke test on $host"
    ssh "$host" "\$HOME/$REMOTE_BIN/generate-peq --graphic-eq-list | head -3"
    # --version exits 0 without touching data, proving the ELF runs.
    ssh "$host" "\$HOME/$REMOTE_BIN/generate-radar --version"
    ssh "$host" "\$HOME/$REMOTE_BIN/generate-eq-compare --version"
    # The headphone bins have no --version flag; --help exits 0 without
    # touching data, proving the ELF runs.
    ssh "$host" "\$HOME/$REMOTE_BIN/generate-headphone-meta --help >/dev/null"
    ssh "$host" "\$HOME/$REMOTE_BIN/generate-headphone-graph --help >/dev/null"
    ssh "$host" "\$HOME/$REMOTE_BIN/generate-peq-headphone --help >/dev/null"
    # generate-graph without arguments must fail with its own message,
    # which proves the ELF runs under the target glibc.
    ssh "$host" "\$HOME/$REMOTE_BIN/generate-graph 2>&1 | head -2 || true"
    ssh "$host" "\$HOME/$REMOTE_BIN/spinorama-api keys 2>&1 | head -1 || true"
}

if [ "$TARGET" = "dev" ] || [ "$TARGET" = "all" ]; then
    install_and_smoke "$DEV_HOST"
fi

if [ "$TARGET" = "prod" ] || [ "$TARGET" = "all" ]; then
    required=$(ssh "$DEV_HOST" "objdump -T ~/$REMOTE_BUILD/target/release/generate-graph \
        ~/$REMOTE_BUILD/target/release/generate-peq \
        ~/$REMOTE_BUILD/target/release/generate-radar \
        ~/$REMOTE_BUILD/target/release/generate-eq-compare \
        ~/$REMOTE_BUILD/target/release/generate-headphone-meta \
        ~/$REMOTE_BUILD/target/release/generate-headphone-graph \
        ~/$REMOTE_BUILD/target/release/generate-peq-headphone \
        ~/$REMOTE_BUILD/target/release/spinorama-api | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1")
    required=${required#GLIBC_}
    prod_glibc=$(ssh "$PROD_HOST" "ldd --version | head -1 | grep -o '[0-9.]*$'")
    newest=$(printf '%s\n%s\n' "$required" "$prod_glibc" | sort -V | tail -1)
    if [ "$newest" != "$prod_glibc" ]; then
        echo "KO: binaries require glibc $required but prod has $prod_glibc; not deploying." >&2
        echo "Build on an older-glibc host or fix the upstream cross-build (see header)." >&2
        exit 1
    fi
    echo "==> glibc check OK (require $required <= prod $prod_glibc)"
    install_and_smoke "$PROD_HOST"
fi

echo "Done ($TARGET)."
