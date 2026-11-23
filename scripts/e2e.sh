#!/usr/bin/env bash
# The tool end to end on Linux, for the e2e job in product.yml. It installs a real tool
# from tests/e2e-project, then checks that:
# - `check --sandbox require` fails without a sandbox even when every tool is up to date,
# - `install --offline` after `check` builds with no network and a read-only CARGO_HOME,
# - a tool that's skipped gets an `up-to-date` record,
# - `install --sandbox require` builds in bwrap.
# It removes and installs bubblewrap with sudo and apt, and needs sfw and jq.
#
# Usage: scripts/e2e.sh <work dir>, with cargo-project-tools on PATH.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
work=$1
rm -rf "$work"
cp -R "$repo/tests/e2e-project" "$work"
cd "$work"

fail() {
    echo "::error::$*" >&2
    exit 1
}
# Fail unless `file` has a record for ripgrep that matches the jq `filter`.
expect() {
    local file=$1 filter=$2
    jq -e "select(.tool == \"ripgrep\" and $filter)" "$file" >/dev/null ||
        fail "no record with $filter in $file: $(cat "$file")"
}
# What the action does: no network, everything read-only but what `install` writes.
isolated() {
    local tools="$work/.project-tools"
    bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp --unshare-net \
        --bind "$tools/targets" "$tools/targets" \
        --bind "$tools/bin" "$tools/bin" \
        --bind "$tools/.crates.toml" "$tools/.crates.toml" \
        --bind "$tools/.crates2.json" "$tools/.crates2.json" \
        -- "$@"
}

echo "::group::Without a sandbox"
if command -v bwrap >/dev/null; then
    sudo apt-get remove -y bubblewrap
fi
cargo project-tools install --no-audit --sandbox off --message-format json \
    >install.jsonl 2>install.log || fail "install failed: $(cat install.log)"
expect install.jsonl '.kind == "built" and .sandbox == "none"'
if grep -q "to your PATH" install.log; then
    fail "cargo asked to add the bin directory to PATH"
fi
if cargo project-tools check --no-audit --sandbox require 2>require.log; then
    fail "check --sandbox require passed without a sandbox"
fi
grep -q "a build sandbox is required" require.log ||
    fail "check failed for another reason: $(cat require.log)"
cargo project-tools check --no-audit --message-format json >check.jsonl
expect check.jsonl '.kind == "up-to-date"'
echo "::endgroup::"

echo "::group::Offline from what check prepared"
sudo apt-get install -y bubblewrap
# Ubuntu's AppArmor stops unprivileged user namespaces, which bwrap needs.
sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
if isolated curl -sS -m 5 https://index.crates.io/config.json; then
    fail "the isolated run has a network"
fi
if isolated touch "${CARGO_HOME:-$HOME/.cargo}/e2e-write"; then
    fail "the isolated run can write to CARGO_HOME"
fi
rm -rf .project-tools/bin/.stamps
cargo project-tools check --no-audit
test -f .project-tools/targets/ripgrep/prepared.json || fail "check prepared nothing"
isolated cargo project-tools install --offline --no-audit --sandbox off \
    --message-format json >offline.jsonl
expect offline.jsonl '.kind == "built"'
test ! -f .project-tools/targets/ripgrep/prepared.json || fail "prepared state used twice"
isolated cargo project-tools install --offline --no-audit --sandbox off \
    --message-format json >again.jsonl
expect again.jsonl '.kind == "up-to-date"'
echo "::endgroup::"

echo "::group::In bwrap"
rm -rf .project-tools/bin/.stamps
cargo project-tools install --no-audit --sandbox require --message-format json >bwrap.jsonl
expect bwrap.jsonl '.kind == "built" and .sandbox == "bwrap"'
echo "::endgroup::"
