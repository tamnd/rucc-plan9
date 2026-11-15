#!/bin/sh
# Build the 9h monitor. On macOS, sign it with the hypervisor entitlement, because
# Hypervisor.framework refuses a process that does not have it.
set -e
cd "$(dirname "$0")"
cargo build --release --locked "$@"
if [ "$(uname -s)" = Darwin ]; then
	codesign -s - -f --entitlements 9h.entitlements target/release/9h
fi
