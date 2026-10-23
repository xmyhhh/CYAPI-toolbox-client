$ErrorActionPreference = 'Stop'
$toolRoot = Join-Path $env:LOCALAPPDATA 'CYAPI-toolbox\build-tools'
if (Test-Path (Join-Path $toolRoot 'cargo\bin\cargo.exe')) {
    $env:CARGO_HOME = Join-Path $toolRoot 'cargo'
    $env:RUSTUP_HOME = Join-Path $toolRoot 'rustup'
    $env:Path = "$(Join-Path $env:CARGO_HOME 'bin');$env:Path"
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'Rust toolchain not found. Run npm run desktop:setup first.'
}
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
try {
    # Bundle only the formats this platform can produce.
    if ($args.Count -gt 0 -and $args[0] -eq 'build' -and ($args -join ' ') -notmatch '--bundles') {
        $args = @($args) + @('--bundles', 'nsis')
    }
    & (Join-Path $projectRoot 'node_modules\.bin\tauri.cmd') @args
    if ($LASTEXITCODE -ne 0) { throw "Tauri exited with code $LASTEXITCODE" }
} finally { Pop-Location }
