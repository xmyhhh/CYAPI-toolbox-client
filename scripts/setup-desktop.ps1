param([switch]$UseMirror)
$ErrorActionPreference = 'Stop'
$toolRoot = Join-Path $env:LOCALAPPDATA 'CYAPI-toolbox\build-tools'
$env:CARGO_HOME = Join-Path $toolRoot 'cargo'
$env:RUSTUP_HOME = Join-Path $toolRoot 'rustup'
if ($UseMirror) {
    $env:RUSTUP_DIST_SERVER = 'https://rsproxy.cn'
    $env:RUSTUP_UPDATE_ROOT = 'https://rsproxy.cn/rustup'
    New-Item -ItemType Directory -Force -Path $env:CARGO_HOME | Out-Null
    $mirrorConfig = @'
[source.crates-io]
replace-with = "rsproxy-sparse"
[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"
[net]
retry = 3
'@
    Set-Content -LiteralPath (Join-Path $env:CARGO_HOME 'config.toml') -Value $mirrorConfig -Encoding utf8
}
New-Item -ItemType Directory -Force -Path $toolRoot | Out-Null
$rustupPath = Join-Path $toolRoot 'rustup-init.exe'
if (-not (Test-Path (Join-Path $env:CARGO_HOME 'bin\rustup.exe'))) {
    $updateRoot = if ($env:RUSTUP_UPDATE_ROOT) { $env:RUSTUP_UPDATE_ROOT } else { 'https://static.rust-lang.org/rustup' }
    & curl.exe --fail --location --retry 3 --output $rustupPath "$updateRoot/dist/x86_64-pc-windows-msvc/rustup-init.exe"
    if ($LASTEXITCODE -ne 0) { throw 'Rustup download failed.' }
    & $rustupPath -y --profile minimal --default-toolchain stable --default-host x86_64-pc-windows-msvc --no-modify-path
} else {
    & (Join-Path $env:CARGO_HOME 'bin\rustup.exe') toolchain install stable --profile minimal
}
if ($LASTEXITCODE -ne 0) { throw 'Rust toolchain installation failed.' }
Write-Host "Local Rust toolchain ready: $toolRoot"
Write-Host 'Build with: npm run desktop:build'
