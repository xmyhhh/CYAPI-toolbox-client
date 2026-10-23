#!/bin/sh
# Installs a local Rust toolchain for the desktop build.
set -e
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tool_root="${HOME}/Library/Caches/CYAPI-toolbox/build-tools"
CARGO_HOME="${tool_root}/cargo"
RUSTUP_HOME="${tool_root}/rustup"
export CARGO_HOME RUSTUP_HOME
mkdir -p "${tool_root}"
if [ ! -x "${CARGO_HOME}/bin/rustup" ]; then
    host="$(uname -m)-apple-darwin"
    update_root="${RUSTUP_UPDATE_ROOT:-https://static.rust-lang.org/rustup}"
    curl --fail --location --retry 3 --output "${tool_root}/rustup-init" \
        "${update_root}/dist/${host}/rustup-init"
    chmod +x "${tool_root}/rustup-init"
    "${tool_root}/rustup-init" -y --profile minimal --default-toolchain stable --no-modify-path
else
    "${CARGO_HOME}/bin/rustup" toolchain install stable --profile minimal
fi
echo "Local Rust toolchain ready: ${tool_root}"
echo 'Build with: npm run desktop:build'
