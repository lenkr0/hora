<#
.SYNOPSIS
    Prüft das Verhalten im Infobereich mit eigenem Datenordner:
    Schließen legt Hora in den Infobereich, ein zweiter Start holt das
    Fenster zurück und beendet sich, --tray startet ohne Fenster.
    Zeigt beim ersten Schließen den Hinweis "Hora läuft im Infobereich weiter".

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\shell-test.ps1 -Exe target\release\hora.exe
#>
param([Parameter(Mandatory)] [string] $Exe)

$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class HoraWindows {
    delegate bool EnumProc(IntPtr hwnd, IntPtr param);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr param);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder text, int size);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int size);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr w, IntPtr l);
    // Hauptfenster (Titel "Hora") bzw. Infobereich-Fenster eines Prozesses
    public static IntPtr Find(uint pid, bool tray) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, p) => {
            uint owner; GetWindowThreadProcessId(h, out owner);
            if (owner != pid) return true;
            var cls = new StringBuilder(256); GetClassName(h, cls, 256);
            var title = new StringBuilder(256); GetWindowText(h, title, 256);
            bool isTray = cls.ToString() == "HoraTrayWindow";
            if (tray ? isTray : (!isTray && title.ToString() == "Hora")) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@

function Visible([int] $id) {
    $h = [HoraWindows]::Find([uint32]$id, $false)
    return ($h -ne [IntPtr]::Zero) -and [HoraWindows]::IsWindowVisible($h)
}
function WaitFor([scriptblock] $condition, [int] $seconds = 15) {
    $until = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $until) { if (& $condition) { return $true }; Start-Sleep -Milliseconds 200 }
    return $false
}

$work = Join-Path $env:TEMP "hora-shell-test-$PID"
$env:HORA_DATA_DIR = $work
$env:HORA_CREDENTIALS = 'memory'
$failed = 0
function Check([bool] $ok, [string] $text) {
    if ($ok) { Write-Host "OK   $text" } else { Write-Host "FEHL $text"; $script:failed++ }
}

$first = $null
try {
    $first = Start-Process $Exe -PassThru
    Check (WaitFor { Visible $first.Id }) 'Fenster erscheint'
    Check (WaitFor { [HoraWindows]::Find([uint32]$first.Id, $true) -ne [IntPtr]::Zero }) 'Symbol im Infobereich angelegt'

    # Schließen (wie das X oben rechts): Fenster weg, Hora läuft weiter
    $main = [HoraWindows]::Find([uint32]$first.Id, $false)
    [void][HoraWindows]::PostMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    Check (WaitFor { -not (Visible $first.Id) } 5) 'Schließen blendet das Fenster aus'
    Start-Sleep -Seconds 1
    Check (-not $first.HasExited) 'Hora läuft im Infobereich weiter'

    # Zweiter Start: holt das Fenster zurück und beendet sich selbst
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $second = Start-Process $Exe -PassThru -Wait
    Check ($second.ExitCode -eq 0 -and $watch.Elapsed.TotalSeconds -lt 5) ("Zweiter Start beendet sich ({0:N1} s)" -f $watch.Elapsed.TotalSeconds)
    Check (WaitFor { Visible $first.Id } 5) 'Erste Instanz zeigt ihr Fenster wieder'
    Stop-Process -Id $first.Id -Force
    $first.WaitForExit()

    # Autostart-Variante: ohne Fenster, nur im Infobereich
    $tray = Start-Process $Exe -ArgumentList '--tray' -PassThru
    Check (WaitFor { [HoraWindows]::Find([uint32]$tray.Id, $true) -ne [IntPtr]::Zero }) '--tray: Symbol im Infobereich'
    Start-Sleep -Seconds 1
    Check (-not (Visible $tray.Id)) '--tray: kein Fenster'
    $second = Start-Process $Exe -PassThru -Wait
    Check (WaitFor { Visible $tray.Id } 5) '--tray: zweiter Start öffnet das Fenster'
    Stop-Process -Id $tray.Id -Force
}
finally {
    Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($Exe)) -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $Exe } | Stop-Process -Force -ErrorAction SilentlyContinue
    Remove-Item Env:HORA_DATA_DIR, Env:HORA_CREDENTIALS -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
if ($failed -gt 0) { throw "$failed Prüfung(en) fehlgeschlagen" }
Write-Host 'Infobereich-Test bestanden'
