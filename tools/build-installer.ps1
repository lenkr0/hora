<#
.SYNOPSIS
    Baut hora.exe (Release) und daraus den Installer Hora-Setup-<Version>.exe
    mit NSIS 3 (makensis im PATH oder unter Programme\NSIS).

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\build-installer.ps1
#>
param([string] $OutDir)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
if (-not $OutDir) { $OutDir = Join-Path $target 'installer' }

$makensis = (Get-Command makensis -ErrorAction SilentlyContinue).Source
if (-not $makensis) {
    $makensis = @("${env:ProgramFiles(x86)}\NSIS\makensis.exe", "$env:ProgramFiles\NSIS\makensis.exe") |
        Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $makensis) { throw 'NSIS nicht gefunden (winget install NSIS.NSIS)' }

$version = [regex]::Match((Get-Content (Join-Path $root 'Cargo.toml') -Raw), '(?m)^version = "([^"]+)"').Groups[1].Value
Push-Location $root
try {
    cargo build --release -p hora-app
    if ($LASTEXITCODE -ne 0) { throw 'cargo build fehlgeschlagen' }
}
finally { Pop-Location }

New-Item -ItemType Directory -Force $OutDir | Out-Null
$exe = Join-Path $target 'release\hora.exe'
$out = Join-Path $OutDir "Hora-Setup-$version.exe"
& $makensis /V2 "/DVERSION=$version" "/DEXE=$exe" "/DOUT=$out" (Join-Path $root 'installer\hora.nsi')
if ($LASTEXITCODE -ne 0) { throw 'makensis fehlgeschlagen' }
Write-Host ("Installer: {0} ({1:N1} MB)" -f $out, ((Get-Item $out).Length / 1MB))
