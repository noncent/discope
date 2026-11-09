#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"
cargo build --release --locked

release_dir="${CARGO_TARGET_DIR:-target}/release"
if [[ "$(uname -s)" == Darwin ]]; then
    destination="$release_dir/Discope.app"
    stage=$(mktemp -d "$release_dir/.discope.XXXXXX")
    trap 'rm -rf "$stage"' EXIT
    bundle="$stage/Discope.app"
    mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
    cp "$release_dir/disk-analyzer" "$bundle/Contents/MacOS/"
    cp packaging/Info.plist "$bundle/Contents/"
    cp packaging/Discope.icns "$bundle/Contents/Resources/"
    codesign --force --sign - "$bundle"
    codesign --verify --strict "$bundle"
    if [[ -e "$destination" ]]; then
        mv "$destination" "$stage/previous.app"
    fi
    if ! mv "$bundle" "$destination"; then
        if [[ -d "$stage/previous.app" ]]; then
            mv "$stage/previous.app" "$destination"
        fi
        exit 1
    fi
    printf 'Open %s\n' "$destination"
else
    printf 'Binary available in %s\n' "$release_dir"
fi
