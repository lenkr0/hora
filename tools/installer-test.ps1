<#
.SYNOPSIS
    Installiert Hora still in einen Testordner, prüft Dateien, Startmenü,
    Apps-Eintrag und Autostart, startet das installierte Programm kurz (mit
    eigenem Datenordner) und deinstalliert wieder. Danach darf nichts übrig
    sein. Der echte Datenordner %LOCALAPPDATA%\Hora bleibt unberührt.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\installer-test.ps1 -Setup target\installer\Hora-Setup-0.5.0.exe
#>
param([Parameter(Mandatory)] [string] $Setup)

$ErrorActionPreference = 'Stop'
$Setup = (Resolve-Path $Setup).Path
$dir = Join-Path $env:TEMP "hora-install-test-$PID"
$data = Join-Path $env:TEMP "hora-install-data-$PID"
$uninstKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Hora'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Hora.lnk'
$failed = 0
function Check([bool] $ok, [string] $text) {
    if ($ok) { Write-Host "OK   $text" } else { Write-Host "FEHL $text"; $script:failed++ }
}
if (Test-Path $uninstKey) { throw 'Hora ist bereits installiert; Test würde die Installation überschreiben' }

try {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process $Setup -ArgumentList '/S', "/D=$dir" -PassThru -Wait
    Check ($p.ExitCode -eq 0) ("Stille Installation ({0:N1} s)" -f $watch.Elapsed.TotalSeconds)
    $exe = Join-Path $dir 'hora.exe'
    Check (Test-Path $exe) 'hora.exe installiert'
    Check ((Test-Path "$dir\LICENSE") -and (Test-Path "$dir\NOTICE.md") -and (Test-Path "$dir\THIRD_PARTY_LICENSES\Calino-LICENSE.txt") -and (Test-Path "$dir\THIRD_PARTY_LICENSES\Rust-Crates.txt")) 'Lizenzen und Hinweise dabei'
    Check ((Get-Item $exe).VersionInfo.ProductName -eq 'Hora') 'Versionsangaben im Programm'

    $link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
    Check ((Test-Path $shortcut) -and $link.TargetPath -eq $exe) 'Startmenü-Eintrag zeigt auf hora.exe'
    $entry = Get-ItemProperty $uninstKey -ErrorAction SilentlyContinue
    Check ($entry.DisplayName -eq 'Hora' -and $entry.DisplayVersion -and $entry.UninstallString -like "*uninstall.exe*") "Eintrag unter Apps (Version $($entry.DisplayVersion), $($entry.EstimatedSize) KB)"
    $run = (Get-ItemProperty $runKey -ErrorAction SilentlyContinue).Hora
    Check ($run -eq "`"$exe`" --tray") 'Autostart mit --tray eingetragen'

    # Installiertes Programm kurz starten (eigener Datenordner), dann per --quit beenden
    $env:HORA_DATA_DIR = $data
    $env:HORA_CREDENTIALS = 'memory'
    $app = Start-Process $exe -ArgumentList '--tray' -PassThru
    Start-Sleep -Seconds 3
    Check (-not $app.HasExited) 'Installiertes Hora startet'
    Start-Process $exe -ArgumentList '--quit' -Wait
    Check ($app.WaitForExit(5000)) 'hora --quit beendet die laufende Instanz'
    Remove-Item Env:HORA_DATA_DIR, Env:HORA_CREDENTIALS

    # Deinstallieren (in place, damit der Aufruf wartet)
    $p = Start-Process "$dir\uninstall.exe" -ArgumentList '/S', "_?=$dir" -PassThru -Wait
    Check ($p.ExitCode -eq 0) 'Stille Deinstallation'
    Remove-Item "$dir\uninstall.exe" -ErrorAction SilentlyContinue
    Check (-not (Test-Path $exe)) 'Programm entfernt'
    Check (-not (Test-Path $shortcut)) 'Startmenü-Eintrag entfernt'
    Check (-not (Test-Path $uninstKey)) 'Apps-Eintrag entfernt'
    Check (-not (Get-ItemProperty $runKey -ErrorAction SilentlyContinue).Hora) 'Autostart entfernt'
    Check (-not (Test-Path 'HKCU:\Software\Classes\AppUserModelId\Hora.Kalender')) 'Anmeldung für Benachrichtigungen entfernt'
}
finally {
    Remove-Item Env:HORA_DATA_DIR, Env:HORA_CREDENTIALS -ErrorAction SilentlyContinue
    if (Test-Path "$dir\uninstall.exe") { Start-Process "$dir\uninstall.exe" -ArgumentList '/S', "_?=$dir" -Wait }
    Remove-Item -Recurse -Force $dir, $data -ErrorAction SilentlyContinue
}
if ($failed -gt 0) { throw "$failed Prüfung(en) fehlgeschlagen" }
Write-Host 'Installer-Test bestanden'
