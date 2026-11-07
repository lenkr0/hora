<#
.SYNOPSIS
    Misst Startzeit und Arbeitsspeicher im Leerlauf.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\measure.ps1 -Exe target\release\hora.exe
    powershell -ExecutionPolicy Bypass -File tools\measure.ps1 -Exe target\release\hora.exe -Backends auto,winit-software,winit-femtovg -Runs 5

.NOTES
    Zeiten ab Prozessstart (inklusive Laden der DLLs):
      main             Beginn von main()
      Ereignisschleife erster Durchlauf der Ereignisschleife, Fenster ist erzeugt
      Erstes Bild      erstes fertig gezeichnetes Bild; melden nur OpenGL-Renderer (femtovg)
    "Privat" ist der private Arbeitssatz, also der Wert in der Spalte
    "Arbeitsspeicher" des Task-Managers. "auto" lässt die App den Renderer wählen.

    Hora läuft dabei mit eigenem, leerem Datenordner (Beispieltermine), nie mit
    dem normalen unter %LOCALAPPDATA%\Hora. -DataDir nimmt stattdessen eine
    Kopie dieses Ordners, z. B. mit Testdaten aus `hora --fill-test-data`
    (Devtools-Build):

        $env:HORA_DATA_DIR = "$env:TEMP\hora-gross"; target\debug\hora.exe --fill-test-data
        powershell -File tools\measure.ps1 -Exe target\release\hora.exe -Backends auto -DataDir $env:TEMP\hora-gross
#>
param(
    [Parameter(Mandatory)] [string] $Exe,
    [string[]] $Backends = @('auto', 'winit-software', 'winit-femtovg'),
    [int] $Runs = 5,
    [int] $IdleSeconds = 10,
    [string] $DataDir
)

$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
$work = Join-Path $env:TEMP "hora-measure-$PID"
$env:HORA_CREDENTIALS = 'memory'
$probe = Join-Path $env:TEMP 'hora-measure.txt'
$rows = @()

function Get-Ms([string] $line, [string] $key) {
    if ($line -match "$key=([\d.]+)") { [double]$Matches[1] } else { $null }
}

foreach ($backend in $Backends) {
    for ($run = 1; $run -le $Runs; $run++) {
        Remove-Item $probe -ErrorAction SilentlyContinue
        # Jeder Lauf mit frischer Kopie der Daten
        Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
        if ($DataDir) { Copy-Item -Recurse $DataDir $work } else { New-Item -ItemType Directory $work | Out-Null }
        $env:HORA_DATA_DIR = $work
        if ($backend -eq 'auto') { Remove-Item Env:SLINT_BACKEND -ErrorAction SilentlyContinue } else { $env:SLINT_BACKEND = $backend }
        $env:HORA_MEASURE = $probe
        $proc = Start-Process -FilePath $Exe -PassThru
        # Die App schreibt die Messdatei drei Sekunden nach dem Start
        $deadline = (Get-Date).AddSeconds(8)
        while (-not (Test-Path $probe) -and -not $proc.HasExited -and (Get-Date) -lt $deadline) {
            Start-Sleep -Milliseconds 100
        }
        Start-Sleep -Seconds ([Math]::Max(0, $IdleSeconds - 3))
        if ($proc.HasExited) {
            $rows += [pscustomobject]@{ Renderer = $backend; Lauf = $run; 'main (ms)' = "abgestürzt, Exitcode $($proc.ExitCode)" }
            continue
        }
        $line = if (Test-Path $probe) { Get-Content $probe -Raw } else { '' }
        $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess=$($proc.Id)"
        $proc.Refresh()
        $rows += [pscustomobject]@{
            Renderer                 = $backend
            Lauf                     = $run
            'main (ms)'              = Get-Ms $line 'main_ms'
            'Ereignisschleife (ms)'  = Get-Ms $line 'event_loop_ms'
            'Erstes Bild (ms)'       = Get-Ms $line 'first_frame_ms'
            'Privat (Task-Manager)'  = '{0:N1} MB' -f ($perf.WorkingSetPrivate / 1MB)
            'Arbeitssatz'            = '{0:N1} MB' -f ($proc.WorkingSet64 / 1MB)
            'Zugesichert'            = '{0:N1} MB' -f ($proc.PrivateMemorySize64 / 1MB)
        }
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 500
    }
}

Remove-Item Env:SLINT_BACKEND, Env:HORA_MEASURE, Env:HORA_DATA_DIR, Env:HORA_CREDENTIALS -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
$columns = 'Renderer', 'Lauf', 'main (ms)', 'Ereignisschleife (ms)', 'Erstes Bild (ms)', 'Privat (Task-Manager)', 'Arbeitssatz', 'Zugesichert'
$rows | Format-Table -Property $columns -AutoSize
