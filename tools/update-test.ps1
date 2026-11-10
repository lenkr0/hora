<#
.SYNOPSIS
    Probt das Update aus der App von Anfang bis Ende: installiert Hora still
    in einen Testordner, bietet auf einem lokalen Server ein "neueres"
    Release an (dasselbe Setup als Version 99.0.0, mit dem echten Schlüssel
    signiert), startet das installierte Hora mit HORA_UPDATE_URL, wartet auf
    den Hinweis in der Titelleiste, klickt ihn und prüft, dass Hora sich
    beendet, das Setup still aktualisiert und Hora neu startet. Danach wird
    alles wieder entfernt; der echte Datenordner bleibt unberührt.

    Braucht den geheimen Schlüssel (.secrets\hora-update-key.txt oder
    Umgebungsvariable HORA_UPDATE_KEY), Python (lokaler Server) und Cargo
    (hora-sign).

    Mit -Tamper wird update.txt nach dem Signieren verändert: Dann darf Hora
    weder etwas laden noch einen Hinweis zeigen.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\update-test.ps1 -Setup target\installer\Hora-Setup-1.2.0.exe
#>
param([Parameter(Mandatory)] [string] $Setup, [switch] $Tamper)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$Setup = (Resolve-Path $Setup).Path
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
# Wie in installer-test.ps1: mit dem langen Pfad von TEMP rechnen
Add-Type -Namespace HoraUpdateTest -Name Path -MemberDefinition @'
[DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
public static extern uint GetLongPathName(string shortPath, System.Text.StringBuilder longPath, uint size);
'@
$buffer = New-Object System.Text.StringBuilder 32768
$temp = if ([HoraUpdateTest.Path]::GetLongPathName($env:TEMP, $buffer, $buffer.Capacity) -gt 0) { $buffer.ToString() } else { $env:TEMP }
$dir = Join-Path $temp "hora update test $PID"
$data = Join-Path $temp "hora-update-data-$PID"
$web = Join-Path $temp "hora-update-web-$PID"
$uninstKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Hora'
$version = '99.0.0'
$failed = 0
function Check([bool] $ok, [string] $text) {
    if ($ok) { Write-Host "OK   $text" } else { Write-Host "FEHL $text"; $script:failed++ }
}
if (Test-Path $uninstKey) { throw 'Hora ist bereits installiert; Test würde die Installation überschreiben' }
if (-not $env:HORA_UPDATE_KEY) {
    $keyFile = Join-Path $root '.secrets\hora-update-key.txt'
    if (-not (Test-Path $keyFile)) { throw "Kein Schlüssel: $keyFile oder HORA_UPDATE_KEY" }
    $env:HORA_UPDATE_KEY = (Get-Content $keyFile -Raw).Trim()
}

function Find-Hora([int] $processId) {
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, 'Hora')
    foreach ($w in [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)) {
        if ($w.Current.ProcessId -eq $processId) { return $w }
    }
    $null
}

$server = $null
$p = $null
try {
    # 1. Installieren
    $s = Start-Process $Setup -ArgumentList '/S', '--lang=en', "/D=$dir" -PassThru -Wait
    Check ($s.ExitCode -eq 0 -and (Test-Path "$dir\hora.exe")) 'Hora still installiert'

    # 2. "Neueres" Release wie bei GitHub: download/v<Version>/… und latest/download/update.txt
    $release = Join-Path $web "download\v$version"
    New-Item -ItemType Directory -Force $release, (Join-Path $web 'latest\download') | Out-Null
    Copy-Item $Setup (Join-Path $release "Hora-Setup-$version.exe")
    Push-Location $root
    try { cargo run -q -p hora-update --bin hora-sign --locked -- manifest (Join-Path $release "Hora-Setup-$version.exe") $release }
    finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw 'hora-sign fehlgeschlagen' }
    if ($Tamper) {
        # Größe nach dem Signieren ändern: die Signatur passt nicht mehr
        $text = (Get-Content "$release\update.txt" -Raw) -replace '(?m)^size (\d+)', 'size 1$1'
        [IO.File]::WriteAllText("$release\update.txt", $text)
    }
    Copy-Item "$release\update.txt" (Join-Path $web 'latest\download\update.txt')

    # 3. Lokaler Server
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $listener.Start(); $port = $listener.LocalEndpoint.Port; $listener.Stop()
    $server = Start-Process python -ArgumentList '-m', 'http.server', "$port", '--bind', '127.0.0.1', '--directory', $web -PassThru -WindowStyle Hidden
    $base = "http://127.0.0.1:$port"
    $up = $false
    for ($i = 0; $i -lt 50 -and -not $up; $i++) {
        try { $up = (Invoke-WebRequest "$base/latest/download/update.txt" -UseBasicParsing -TimeoutSec 2).StatusCode -eq 200 } catch { Start-Sleep -Milliseconds 200 }
    }
    Check $up 'Lokaler Update-Server läuft'

    # 4. Installiertes Hora mit eigenem Datenordner und dem lokalen Server starten
    $env:HORA_DATA_DIR = $data; $env:HORA_CREDENTIALS = 'memory'; $env:HORA_LANGUAGE = 'en'
    $env:HORA_UPDATE_URL = $base; $env:HORA_UPDATE_DELAY = '2'
    $p = Start-Process "$dir\hora.exe" -PassThru

    # 5. Hinweis in der Titelleiste abwarten (prüft, lädt, kontrolliert die Signatur)
    $label = "Hora $version is ready · Restart to update"
    $labelCond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $label)
    $notice = $null
    $wait = if ($Tamper) { 15 } else { 90 }
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while (-not $notice -and $watch.Elapsed.TotalSeconds -lt $wait) {
        Start-Sleep -Milliseconds 500
        $win = Find-Hora $p.Id
        if ($win) { $notice = $win.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $labelCond) }
    }
    if ($Tamper) {
        Check ($null -eq $notice) 'Kein Hinweis bei verändertem update.txt'
        Check ((Get-ChildItem "$data\updates" -ErrorAction SilentlyContinue | Measure-Object).Count -eq 0) 'Nichts geladen'
        Check (-not $p.HasExited) 'Hora läuft normal weiter'
    }
    else {
        Check ($null -ne $notice) ("Hinweis in der Titelleiste ({0:N1} s)" -f $watch.Elapsed.TotalSeconds)
        Check (Test-Path "$data\updates\Hora-Setup-$version.exe") 'Setup geladen und geprüft'
        if (-not $notice) { throw 'Kein Update-Hinweis, Test abgebrochen' }

        # 6. Klicken: Hora beendet sich, das Setup aktualisiert still und startet Hora neu
        $notice.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
        Check ($p.WaitForExit(15000)) 'Hora beendet sich für das Update'
        $new = $null
        $watch.Restart()
        while (-not $new -and $watch.Elapsed.TotalSeconds -lt 60) {
            Start-Sleep -Milliseconds 500
            $new = Get-Process hora -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $p.Id -and $_.Path -eq "$dir\hora.exe" } | Select-Object -First 1
        }
        Check ($null -ne $new) ("Setup aktualisiert und startet Hora neu ({0:N1} s)" -f $watch.Elapsed.TotalSeconds)
        $log = Join-Path $env:TEMP 'Hora-Setup.log'
        $last = if (Test-Path $log) { Get-Content $log -Tail 3 | Out-String } else { '' }
        Check ($last -like "*Stille Installation nach $dir*") 'Setup hat in den bisherigen Ordner installiert'
        $entry = Get-ItemProperty $uninstKey -ErrorAction SilentlyContinue
        Check ($entry.InstallLocation -eq $dir) 'Eintrag unter Apps zeigt weiter auf den Ordner'
    }
}
finally {
    # Beenden mit demselben Datenordner (die laufende Instanz hängt daran)
    if (Test-Path "$dir\hora.exe") { Start-Process "$dir\hora.exe" -ArgumentList '--quit' -Wait }
    Start-Sleep -Milliseconds 800
    Get-Process hora -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq "$dir\hora.exe" } | Stop-Process -Force
    if ($server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force }
    Remove-Item Env:HORA_DATA_DIR, Env:HORA_CREDENTIALS, Env:HORA_LANGUAGE, Env:HORA_UPDATE_URL, Env:HORA_UPDATE_DELAY -ErrorAction SilentlyContinue
    if (Test-Path "$dir\uninstall.exe") {
        Start-Process "$dir\uninstall.exe" -ArgumentList '/S', "_?=$dir" -Wait
        Remove-Item "$dir\uninstall.exe" -ErrorAction SilentlyContinue
    }
    Remove-Item -Recurse -Force $dir, $data, $web -ErrorAction SilentlyContinue
}
Check (-not (Test-Path $uninstKey)) 'Wieder deinstalliert'
if ($failed -gt 0) { throw "$failed Prüfung(en) fehlgeschlagen" }
Write-Host 'Update-Test bestanden'
