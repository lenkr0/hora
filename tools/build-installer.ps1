<#
.SYNOPSIS
    Baut hora.exe und das Setup-Programm (Release) und packt daraus den
    Installer Hora-Setup-<Version>.exe. Kein NSIS mehr nötig: das Setup ist
    crates\setup (Rust + Slint), die Programmdateien hängen xz-komprimiert an
    (LZMA2 mit x86-Filter).

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\build-installer.ps1
#>
param([string] $OutDir)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
if (-not $OutDir) { $OutDir = Join-Path $target 'installer' }

$version = [regex]::Match((Get-Content (Join-Path $root 'Cargo.toml') -Raw), '(?m)^version = "([^"]+)"').Groups[1].Value
Push-Location $root
try {
    cargo build --release -p hora-app --locked
    if ($LASTEXITCODE -ne 0) { throw 'cargo build (hora-app) fehlgeschlagen' }
    # Eigenes Profil: auf Größe optimiert (Cargo.toml, [profile.setup])
    cargo build --profile setup -p hora-setup --locked
    if ($LASTEXITCODE -ne 0) { throw 'cargo build (hora-setup) fehlgeschlagen' }
}
finally { Pop-Location }

# Was installiert wird: Programm, Lizenz, Hinweise, Lizenztexte
$stage = Join-Path $target 'setup-payload'
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$stage\THIRD_PARTY_LICENSES" | Out-Null
Copy-Item (Join-Path $target 'release\hora.exe') $stage
Copy-Item (Join-Path $root 'LICENSE'), (Join-Path $root 'NOTICE.md') $stage
Copy-Item (Join-Path $root 'THIRD_PARTY_LICENSES\*.txt') "$stage\THIRD_PARTY_LICENSES"

New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
$out = Join-Path $OutDir "Hora-Setup-$version.exe"
$setup = Join-Path $target 'setup\hora-setup.exe'
$summary = & $setup --pack $stage $out | Out-String
if ($LASTEXITCODE -ne 0 -or -not (Test-Path $out)) { throw "Packen fehlgeschlagen: $summary" }
Write-Host $summary.Trim()
Write-Host ("Installer: {0} ({1:N1} MB)" -f $out, ((Get-Item $out).Length / 1MB))
