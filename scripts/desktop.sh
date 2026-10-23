#!/bin/sh
# Runs the Tauri CLI with the local Rust toolchain if one is present.
set -e
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tool_root="${HOME}/Library/Caches/CYAPI-toolbox/build-tools"
if [ -x "${tool_root}/cargo/bin/cargo" ]; then
    CARGO_HOME="${tool_root}/cargo"
    RUSTUP_HOME="${tool_root}/rustup"
    export CARGO_HOME RUSTUP_HOME
    PATH="${CARGO_HOME}/bin:${PATH}"
    export PATH
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo 'Rust toolchain not found. Run npm run desktop:setup first.' >&2
    exit 1
fi
cd "${project_root}"
# Bundle only the formats this platform can produce.
if [ "${1:-}" = "build" ]; then
    case " $* " in
        *' --bundles '*) ;;
        *) set -- "$@" --bundles app,dmg ;;
    esac
fi
exec node_modules/.bin/tauri "$@"
