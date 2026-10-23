param([switch]$PortableOnly)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$releaseRoot = Join-Path $projectRoot 'release'
$nativeRoot = Join-Path $projectRoot 'src-tauri\target\release'
$version = (Get-Content -LiteralPath (Join-Path $projectRoot 'package.json') -Raw | ConvertFrom-Json).version
New-Item -ItemType Directory -Force -Path $releaseRoot | Out-Null
$versionedPortable = Join-Path $releaseRoot "CYAPI-Toolbox_${version}.exe"
Copy-Item -LiteralPath (Join-Path $nativeRoot 'cyapi-toolbox.exe') -Destination $versionedPortable -Force
try {
    Copy-Item -LiteralPath $versionedPortable -Destination (Join-Path $releaseRoot 'CYAPI-Toolbox.exe') -Force
} catch [System.IO.IOException] {
    if (($_.Exception.HResult -band 0xFFFF) -notin 32, 33) { throw }
    Write-Host "CYAPI-Toolbox.exe is in use. Use the new executable: $versionedPortable"
}
if (-not $PortableOnly) {
    $setup = Join-Path $nativeRoot "bundle\nsis\CYAPI Toolbox_${version}_x64-setup.exe"
    Copy-Item -LiteralPath $setup -Destination (Join-Path $releaseRoot "CYAPI-Toolbox_${version}_x64-setup.exe") -Force
}
$localLauncher = Join-Path $releaseRoot "CYAPI-Toolbox_${version}-local.cmd"
@"
@echo off
set "CYAPI_TOOLBOX_SERVER_URL=http://127.0.0.1:47821"
start "" "%~dp0CYAPI-Toolbox_${version}.exe"
"@ | Set-Content -LiteralPath $localLauncher -Encoding Ascii
Write-Host "Build artifacts: $releaseRoot"
