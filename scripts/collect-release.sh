#!/bin/sh
# Collects the macOS build artifacts into release/
set -e
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
release_root="${project_root}/release"
native_root="${project_root}/src-tauri/target/release"
version=$(node -p "require('${project_root}/package.json').version")
mkdir -p "${release_root}"

# The unsigned application bundle is the portable copy.
if [ -d "${native_root}/bundle/macos/CYAPI Toolbox.app" ]; then
    rm -rf "${release_root}/CYAPI Toolbox.app"
    cp -R "${native_root}/bundle/macos/CYAPI Toolbox.app" "${release_root}/CYAPI Toolbox.app"
    # The bundle is signed with an ad-hoc signature when the build did not
    # sign it. A bundle without a signature is reported as damaged by
    # Gatekeeper as soon as it is opened from a download.
    if ! codesign --verify --deep --strict "${release_root}/CYAPI Toolbox.app" >/dev/null 2>&1; then
        echo "Signing the application bundle with an ad-hoc signature..."
        codesign --force --deep --sign - "${release_root}/CYAPI Toolbox.app"
    fi
    # A bundle that does not verify is never published.
    codesign --verify --deep --strict "${release_root}/CYAPI Toolbox.app" \
        || { echo "The application bundle is not signed correctly." >&2; exit 1; }
fi

# The plain executable is useful for smoke tests and portable copies.
if [ -f "${native_root}/cyapi-toolbox" ]; then
    cp -f "${native_root}/cyapi-toolbox" "${release_root}/cyapi-toolbox"
    chmod +x "${release_root}/cyapi-toolbox"
fi

# The disk image is the installer copy.
dmg="${native_root}/bundle/dmg/CYAPI Toolbox_${version}_aarch64.dmg"
if [ ! -f "${dmg}" ]; then
    dmg=$(find "${native_root}/bundle/dmg" -maxdepth 1 -name '*.dmg' -print 2>/dev/null | head -n 1 || true)
fi
if [ -n "${dmg}" ] && [ -f "${dmg}" ]; then
    cp -f "${dmg}" "${release_root}/CYAPI-Toolbox_${version}-macos.dmg"
fi

# The zipped application bundle is the portable copy for macOS.
app_zip="${release_root}/CYAPI-Toolbox_${version}-macos.zip"
if [ -d "${release_root}/CYAPI Toolbox.app" ] && command -v ditto >/dev/null 2>&1; then
    rm -f "${app_zip}"
    ditto -c -k --sequesterRsrc --keepParent \
        "${release_root}/CYAPI Toolbox.app" "${app_zip}"
fi

# A local launcher points the client at a locally running catalog server.
local_launcher="${release_root}/CYAPI-Toolbox_${version}-local.command"
cat > "${local_launcher}" <<EOF
#!/bin/sh
export CYAPI_TOOLBOX_SERVER_URL=http://127.0.0.1:47821
exec "${release_root}/CYAPI Toolbox.app/Contents/MacOS/cyapi-toolbox" "\$@"
EOF
chmod +x "${local_launcher}"

echo "Build artifacts: ${release_root}"
