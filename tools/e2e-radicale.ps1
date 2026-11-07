<#
.SYNOPSIS
    Ende-zu-Ende-Test: Radicale im Container mit Testterminen der aktuellen
    Woche, dann verbindet sich Hora über denselben Weg wie der Kontodialog.
    Berührt weder deine echten Daten noch den Windows-Passwortspeicher.

.EXAMPLE
    cargo build -p hora-app --features devtools
    powershell -ExecutionPolicy Bypass -File tools\e2e-radicale.ps1 -Exe target\debug\hora.exe -Snapshot e2e.png
#>
param(
    [Parameter(Mandatory)] [string] $Exe,
    [string] $Snapshot = 'e2e.png',
    [string] $Theme = 'light',
    # Anlegen, Verschieben, Löschen, Serien bearbeiten und Ergebnis auf dem Server prüfen
    [switch] $EditTest,
    # Ordner für Screenshots von Serien-Dialog, Bearbeiten-Dialog und Vorschau (nach -EditTest)
    [string] $Shots
)

$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
$user = 'test'
$password = 'geheim-123'
$name = "hora-e2e-radicale-$PID"
$work = Join-Path $env:TEMP $name
New-Item -ItemType Directory -Force $work | Out-Null
$utf8 = New-Object System.Text.UTF8Encoding $false

function Write-File([string] $path, [string] $text) { [IO.File]::WriteAllText($path, $text, $utf8) }

function Invoke-Dav([string] $method, [string] $url, [string] $file, [string] $type) {
    $code = & curl.exe -s -o NUL -w '%{http_code}' -u "${user}:$password" -X $method -H "Content-Type: $type" --data-binary "@$file" $url
    if ($code -notmatch '^2') { throw "$method $url -> HTTP $code" }
}

try {
    docker create --name $name -p 127.0.0.1::5232 `
        -e RADICALE_CONFIG_AUTH_TYPE=htpasswd `
        -e RADICALE_CONFIG_AUTH_HTPASSWD_FILENAME=/tmp/users `
        -e RADICALE_CONFIG_AUTH_HTPASSWD_ENCRYPTION=plain `
        tomsquest/docker-radicale:3.9.0.0 | Out-Null
    Write-File "$work\users" "${user}:$password`n"
    docker cp "$work\users" "${name}:/tmp/users" | Out-Null
    docker start $name | Out-Null
    $port = (docker port $name 5232 | Select-Object -First 1).Split(':')[-1]
    $base = "http://127.0.0.1:$port"
    for ($i = 0; $i -lt 60; $i++) {
        $code = & curl.exe -s -o NUL -w '%{http_code}' -u "${user}:$password" "$base/$user/"
        if ($code -match '^[2-4]') { break }
        Start-Sleep -Milliseconds 500
    }

    # Zwei Kalender
    foreach ($cal in @(@{slug = 'privat'; name = 'Privat'; color = '#34A853'}, @{slug = 'arbeit'; name = 'Arbeit'; color = '#4285F4'})) {
        Write-File "$work\mk.xml" @"
<?xml version="1.0" encoding="utf-8"?>
<c:mkcalendar xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/"><d:set><d:prop><d:displayname>$($cal.name)</d:displayname><a:calendar-color>$($cal.color)FF</a:calendar-color><c:supported-calendar-component-set><c:comp name="VEVENT"/><c:comp name="VTODO"/></c:supported-calendar-component-set></d:prop></d:set></c:mkcalendar>
"@
        Invoke-Dav 'MKCALENDAR' "$base/$user/$($cal.slug)/" "$work\mk.xml" 'application/xml'
    }
    # Ein Kalender nur für Journaleinträge (wie Baïkal mit „Notes“); Arbeit nimmt auch Journale
    Write-File "$work\mk.xml" @"
<?xml version="1.0" encoding="utf-8"?>
<c:mkcalendar xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/"><d:set><d:prop><d:displayname>Tagebuch</d:displayname><a:calendar-color>#8A6FB0FF</a:calendar-color><c:supported-calendar-component-set><c:comp name="VJOURNAL"/></c:supported-calendar-component-set></d:prop></d:set></c:mkcalendar>
"@
    Invoke-Dav 'MKCALENDAR' "$base/$user/tagebuch/" "$work\mk.xml" 'application/xml'
    Write-File "$work\mk.xml" @"
<?xml version="1.0" encoding="utf-8"?>
<d:propertyupdate xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:set><d:prop><c:supported-calendar-component-set><c:comp name="VEVENT"/><c:comp name="VTODO"/><c:comp name="VJOURNAL"/></c:supported-calendar-component-set></d:prop></d:set></d:propertyupdate>
"@
    Invoke-Dav 'PROPPATCH' "$base/$user/arbeit/" "$work\mk.xml" 'application/xml'

    # Termine relativ zum Montag dieser Woche
    $today = Get-Date
    $monday = $today.Date.AddDays(-(([int]$today.DayOfWeek + 6) % 7))
    function D([int] $offset) { $monday.AddDays($offset).ToString('yyyyMMdd') }
    function Ev([string] $uid, [string] $body) {
        "BEGIN:VCALENDAR`r`nVERSION:2.0`r`nPRODID:-//Hora//E2E//DE`r`nBEGIN:VEVENT`r`nUID:$uid`r`nDTSTAMP:20261104T120000Z`r`n$body`r`nEND:VEVENT`r`nEND:VCALENDAR`r`n"
    }
    $events = @(
        @{cal = 'privat'; file = 'mathe.ics'; ics = Ev 'mathe' "DTSTART;TZID=Europe/Berlin:$(D -14)T080000`r`nDTEND;TZID=Europe/Berlin:$(D -14)T093000`r`nRRULE:FREQ=WEEKLY;BYDAY=MO`r`nSUMMARY:Mathe`r`nLOCATION:Raum 204"},
        @{cal = 'privat'; file = 'zahnarzt.ics'; ics = Ev 'zahnarzt' "DTSTART;TZID=Europe/Berlin:$(D 1)T150000`r`nDTEND;TZID=Europe/Berlin:$(D 1)T160000`r`nSUMMARY:Zahnarzt`r`nLOCATION:Praxis Dr. Weber"},
        @{cal = 'privat'; file = 'urlaub.ics'; ics = Ev 'urlaub' "DTSTART;VALUE=DATE:$(D 4)`r`nDTEND;VALUE=DATE:$(D 7)`r`nSUMMARY:Kurzurlaub Ostsee"},
        @{cal = 'privat'; file = 'markt.ics'; ics = Ev 'markt' "DTSTART:$(D 5)T110000`r`nDTEND:$(D 5)T123000`r`nSUMMARY:Wochenmarkt (schwebende Zeit)"},
        @{cal = 'arbeit'; file = 'ny.ics'; ics = Ev 'ny' "DTSTART;TZID=America/New_York:$(D 2)T100000`r`nDTEND;TZID=America/New_York:$(D 2)T110000`r`nSUMMARY:Call New York (10 Uhr dort)"},
        @{cal = 'arbeit'; file = 'nacht.ics'; ics = Ev 'nacht' "DTSTART;TZID=Europe/Berlin:$(D 3)T220000`r`nDTEND;TZID=Europe/Berlin:$(D 4)T060000`r`nSUMMARY:Nachtschicht"},
        @{cal = 'arbeit'; file = 'konferenz.ics'; ics = Ev 'konferenz' "DTSTART;TZID=Europe/Berlin:$(D 1)T090000`r`nDTEND;TZID=Europe/Berlin:$(D 3)T170000`r`nSUMMARY:Konferenz"},
        @{cal = 'arbeit'; file = 'a.ics'; ics = Ev 'projekt-a' "DTSTART;TZID=Europe/Berlin:$(D 2)T140000`r`nDTEND;TZID=Europe/Berlin:$(D 2)T153000`r`nSUMMARY:Projekt A"},
        @{cal = 'arbeit'; file = 'b.ics'; ics = Ev 'projekt-b' "DTSTART;TZID=Europe/Berlin:$(D 2)T143000`r`nDTEND;TZID=Europe/Berlin:$(D 2)T150000`r`nSUMMARY:Projekt B"},
        # Tägliches Standup Mo–Fr, Mittwoch verschoben, Donnerstag ausgelassen
        @{cal = 'arbeit'; file = 'standup.ics'; ics = "BEGIN:VCALENDAR`r`nVERSION:2.0`r`nPRODID:-//Hora//E2E//DE`r`nBEGIN:VEVENT`r`nUID:standup`r`nDTSTAMP:20261104T120000Z`r`nDTSTART;TZID=Europe/Berlin:$(D 0)T093000`r`nDTEND;TZID=Europe/Berlin:$(D 0)T094500`r`nRRULE:FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR`r`nEXDATE;TZID=Europe/Berlin:$(D 3)T093000`r`nSUMMARY:Standup`r`nEND:VEVENT`r`nBEGIN:VEVENT`r`nUID:standup`r`nDTSTAMP:20261104T120000Z`r`nRECURRENCE-ID;TZID=Europe/Berlin:$(D 2)T093000`r`nDTSTART;TZID=Europe/Berlin:$(D 2)T110000`r`nDTEND;TZID=Europe/Berlin:$(D 2)T111500`r`nSUMMARY:Standup (verschoben)`r`nEND:VEVENT`r`nEND:VCALENDAR`r`n"}
    )
    if ($EditTest) {
        # Beginnt in 10 Minuten, Erinnerung 15 Minuten vorher: sofort fällig
        $soon = (Get-Date).AddMinutes(10)
        $s = $soon.ToString('yyyyMMddTHHmm00'); $e = $soon.AddMinutes(30).ToString('yyyyMMddTHHmm00')
        $events += @{cal = 'privat'; file = 'erinnerung.ics'; ics = Ev 'erinnerung' "DTSTART;TZID=Europe/Berlin:$s`r`nDTEND;TZID=Europe/Berlin:$e`r`nSUMMARY:Erinnerungstest`r`nLOCATION:Testraum`r`nBEGIN:VALARM`r`nACTION:DISPLAY`r`nDESCRIPTION:Erinnerungstest`r`nTRIGGER:-PT15M`r`nEND:VALARM"}
    }
    # Aufgaben: überfällig, ohne Datum, mit Teilaufgabe, wöchentlich wiederkehrend
    function Todo([string] $uid, [string] $body) {
        "BEGIN:VCALENDAR`r`nVERSION:2.0`r`nPRODID:-//Hora//E2E//DE`r`nBEGIN:VTODO`r`nUID:$uid`r`nDTSTAMP:20261104T120000Z`r`n$body`r`nEND:VTODO`r`nEND:VCALENDAR`r`n"
    }
    $events += @(
        @{cal = 'privat'; file = 'steuer.ics'; ics = Todo 'steuer' "SUMMARY:Steuererklärung`r`nDUE;VALUE=DATE:$(D -3)`r`nPRIORITY:1`r`nSTATUS:NEEDS-ACTION"},
        @{cal = 'privat'; file = 'einkauf.ics'; ics = Todo 'einkauf' "SUMMARY:Einkaufsliste schreiben`r`nDESCRIPTION:Brot\, Käse\, Äpfel"},
        @{cal = 'privat'; file = 'umzug.ics'; ics = Todo 'umzug' "SUMMARY:Umzug planen`r`nDUE;VALUE=DATE:$(D 20)"},
        @{cal = 'privat'; file = 'kartons.ics'; ics = Todo 'kartons' "SUMMARY:Kartons kaufen`r`nRELATED-TO;RELTYPE=PARENT:umzug`r`nDUE;VALUE=DATE:$(D 5)"},
        @{cal = 'privat'; file = 'altpapier.ics'; ics = Todo 'altpapier' "SUMMARY:Altpapier raus`r`nSTATUS:COMPLETED`r`nCOMPLETED:20261102T080000Z"},
        @{cal = 'privat'; file = 'sport.ics'; ics = Todo 'sport' "SUMMARY:Sport`r`nDTSTART;VALUE=DATE:$(D 1)`r`nDUE;VALUE=DATE:$(D 1)`r`nRRULE:FREQ=WEEKLY`r`nSTATUS:NEEDS-ACTION"}
    )
    if ($EditTest) {
        # Seit 5 Minuten fällig, ohne eigene Erinnerung: Erinnerung zur Fälligkeit
        $paket = (Get-Date).AddMinutes(-5).ToString('yyyyMMddTHHmm00')
        $events += @{cal = 'privat'; file = 'paket.ics'; ics = Todo 'paket' "SUMMARY:Paket abholen`r`nDUE;TZID=Europe/Berlin:$paket`r`nSTATUS:NEEDS-ACTION"}
    }
    # Journaleintrag eines anderen Programms: Uhrzeit, mehrzeiliger Text, Fremdes.
    # (Radicale nimmt nur eine DESCRIPTION je VJOURNAL an, RFC 5545 erlaubt mehrere.)
    $events += @{cal = 'tagebuch'; file = 'fremd.ics'; ics = "BEGIN:VCALENDAR`r`nVERSION:2.0`r`nPRODID:-//Anderes Programm//DE`r`nBEGIN:VJOURNAL`r`nUID:fremd-journal`r`nDTSTAMP:20261101T080000Z`r`nDTSTART;TZID=Europe/Berlin:$(D 1)T213000`r`nSUMMARY:Notiz von woanders`r`nDESCRIPTION:Teil eins\n\nTeil zwei`r`nCATEGORIES:alt,fremd`r`nX-FREMD:bleibt`r`nEND:VJOURNAL`r`nEND:VCALENDAR`r`n"}
    foreach ($e in $events) {
        Write-File "$work\$($e.file)" $e.ics
        Invoke-Dav 'PUT' "$base/$user/$($e.cal)/$($e.file)" "$work\$($e.file)" 'text/calendar; charset=utf-8'
    }
    Write-Host "Radicale unter $base mit $($events.Count) Terminen"

    $env:HORA_DATA_DIR = "$work\data"
    $env:HORA_CREDENTIALS = 'memory'
    $env:HORA_THEME = $Theme
    $editArgs = @()
    if ($EditTest) { $editArgs += '--edit-test', '--reminder-check' }
    $firstRun = & $Exe --size 1440x900 --connect "$base/" $user $password @editArgs --snapshot $Snapshot
    $firstRun | ForEach-Object { Write-Host $_ }
    Write-Host "Zweiter Start ohne Netz-Sync (Passwort nur im Speicher des ersten Laufs), Daten aus dem Cache:"
    & $Exe --size 1440x900 --connect "$base/" $user $password

    if ($EditTest) {
        # Stand auf dem Server prüfen: alle Termine beider Kalender lesen
        # Erster VEVENT je Ressource (Titel, Beginn) und der ganze Inhalt, entfaltet
        $found = @{}
        $files = @()
        # Alle Kalender, auch den über Hora angelegten Journal-Kalender
        $homeListing = & curl.exe -s -u "${user}:$password" -X PROPFIND -H 'Depth: 1' "$base/$user/"
        $calendars = @([regex]::Matches($homeListing, '<[^>]*href>/' + $user + '/([^/<]+)/</') | ForEach-Object { $_.Groups[1].Value })
        $created = @()
        foreach ($cal in $calendars | Where-Object { $_ -like 'hora-journal-*' }) {
            $props = & curl.exe -s -u "${user}:$password" -X PROPFIND -H 'Depth: 0' "$base/$user/$cal/"
            if ($props -match 'Journal' -and $props -match 'name="VJOURNAL"' -and $props -notmatch 'name="VEVENT"') { $created += $cal }
        }
        foreach ($cal in 'privat', 'arbeit', 'tagebuch') {
            $listing = & curl.exe -s -u "${user}:$password" -X PROPFIND -H 'Depth: 1' "$base/$user/$cal/"
            foreach ($m in [regex]::Matches($listing, '<[^>]*href>([^<]+\.ics)</')) {
                $ics = ((& curl.exe -s -u "${user}:$password" "$base$($m.Groups[1].Value)") -join "`n") -replace "`r", '' -replace "`n[ `t]", ''
                # Nur der erste VEVENT (bzw. VTODO, VJOURNAL) zählt, nicht DTSTART der VTIMEZONE
                $first = @('BEGIN:VEVENT', 'BEGIN:VTODO', 'BEGIN:VJOURNAL' | ForEach-Object { $ics.IndexOf($_) } | Where-Object { $_ -ge 0 } | Sort-Object)[0]
                $vevent = $ics.Substring([Math]::Max(0, [int]$first))
                $summary = [regex]::Match($vevent, '(?m)^SUMMARY[^:]*:(.*)$').Groups[1].Value.Trim()
                $start = [regex]::Match($vevent, '(?m)^DTSTART[^:]*:(.*)$').Groups[1].Value.Trim()
                $found[$summary] = $start
                $files += @{ name = ($m.Groups[1].Value -split '/')[-1]; summary = $summary; start = $start; ics = $ics; cal = $cal }
            }
        }
        Write-Host "Auf dem Server:"; $files | Sort-Object { $_.summary } | ForEach-Object { Write-Host "  $($_.summary) | $($_.start) | $($_.name)" }
        $standup = ($files | Where-Object { $_.name -eq 'standup.ics' }).ics
        $mathe = ($files | Where-Object { $_.name -eq 'mathe.ics' }).ics
        $split = $files | Where-Object { $_.name -ne 'standup.ics' -and $_.summary -eq 'Team-Standup' }
        $lauf = ($files | Where-Object { $_.summary -eq 'Lauftreff' }).ics
        function Ics([string] $summary) { (@($files | Where-Object { $_.summary -eq $summary }) | Select-Object -First 1).ics }
        $sport = ($files | Where-Object { $_.name -eq 'sport.ics' }).ics
        $bericht = [regex]::Match((Ics 'Bericht schreiben'), '(?m)^UID:(.+)$').Groups[1].Value.Trim()
        $now = Get-Date
        $tomorrow = $now.AddDays(1).ToString('yyyyMMdd')
        $nextMonday = $now.AddDays((8 - [int]$now.DayOfWeek) % 7).ToString('yyyyMMdd')
        $nextFriday = $now.AddDays((12 - [int]$now.DayOfWeek) % 7).ToString('yyyyMMdd')
        $arbeit = @($files | Where-Object { $_.cal -eq 'arbeit' } | ForEach-Object { $_.summary })
        $tagebuch = @($files | Where-Object { $_.cal -eq 'tagebuch' } | ForEach-Object { $_.summary })
        $fremd = Ics 'Notiz von woanders (bearbeitet)'
        $schnee = Ics 'Erster Schnee'
        $tippen = Ics 'Tippen beim Hochladen'
        $todayStamp = $now.ToString('yyyyMMdd')
        $checks = @(
            @{ ok = $found.ContainsKey('Neu aus Hora') -and $found['Neu aus Hora'] -like '*T140000'; text = 'Neuer Termin angelegt (14:00)' },
            @{ ok = $found['Zahnarzt'] -like '*T160000'; text = 'Zahnarzt auf 16:00 verschoben' },
            @{ ok = -not $found.ContainsKey('Projekt B'); text = 'Projekt B gelöscht' },
            @{ ok = $found['Team-Standup'] -and -not $found.ContainsKey('Standup'); text = 'Serie für alle Termine in Team-Standup umbenannt' },
            @{ ok = $standup -match '(?m)^SUMMARY:Standup \(verschoben\)$'; text = 'Eigens benannte Ausnahme behält ihren Titel' },
            @{ ok = $standup -match "(?m)^EXDATE[^:]*:.*$(D 1)T093000"; text = 'Nur dieser: Standup am Dienstag ausgelassen (EXDATE)' },
            @{ ok = $standup -match '(?m)^RRULE:.*UNTIL=' -and @($split).Count -eq 1 -and $split.start -like "$(D 4)T100000" -and $split.ics -match '(?m)^RRULE:'; text = 'Dieser und alle folgenden: Serie ab Freitag 10:00 geteilt' },
            @{ ok = $mathe -match "(?m)^RECURRENCE-ID[^:]*:$(D 0)T080000$" -and $mathe -match "(?m)^DTSTART[^:]*:$(D 0)T090000$"; text = 'Nur dieser: Mathe diese Woche auf 9:00 (RECURRENCE-ID)' },
            @{ ok = $lauf -match '(?m)^RRULE:FREQ=WEEKLY;BYDAY=TU,TH;COUNT=4$' -and $found['Lauftreff'] -like "$(D 1)T180000"; text = 'Neue Serie dienstags und donnerstags, 4-mal' },
            @{ ok = $lauf -match '(?m)^TRIGGER:-PT15M$'; text = 'Neue Serie mit Standarderinnerung (15 Minuten vorher)' },
            @{ ok = ($firstRun -join "`n") -match 'erinnerungen gezeigt: ([1-9]\d*), im infocenter: \1'; text = 'Fällige Erinnerung als Windows-Benachrichtigung im Infocenter' },
            @{ ok = ($firstRun -join "`n") -match '(?m)^erinnerung: Paket abholen$'; text = 'Erinnerung zur Fälligkeit einer Aufgabe' },
            @{ ok = (Ics 'Milch kaufen') -match '(?m)^BEGIN:VTODO$' -and (Ics 'Milch kaufen') -match '(?m)^STATUS:NEEDS-ACTION$'; text = 'Aufgabe per Schnelleingabe angelegt' },
            @{ ok = (Ics 'Steuererklärung') -match '(?m)^STATUS:COMPLETED$' -and (Ics 'Steuererklärung') -match '(?m)^COMPLETED:\d{8}T\d{6}Z$'; text = 'Überfällige Aufgabe abgehakt' },
            @{ ok = $sport -match "(?m)^RECURRENCE-ID;VALUE=DATE:$(D 1)$" -and $sport -match '(?m)^STATUS:COMPLETED$' -and ([regex]::Matches($sport, '(?m)^RRULE:').Count -eq 1); text = 'Wiederkehrende Aufgabe: ein Vorkommen erledigt, Serie unverändert' },
            @{ ok = (Ics 'Umzug planen') -match '(?m)^STATUS:COMPLETED$' -and (Ics 'Kartons kaufen') -match '(?m)^STATUS:COMPLETED$'; text = 'Übergeordnete Aufgabe erledigt nimmt Teilaufgabe mit' },
            @{ ok = (Ics 'Bericht schreiben') -match '(?m)^DUE;TZID=Europe/Berlin:\d{8}T180000$' -and (Ics 'Bericht schreiben') -match '(?m)^PRIORITY:1$'; text = 'Aufgabe über den Dialog mit Fälligkeit und Priorität' },
            @{ ok = (Ics 'Physik') -match '(?m)^RRULE:FREQ=WEEKLY;BYDAY=MO$' -and (Ics 'Physik') -match "(?m)^DTSTART;TZID=Europe/Berlin:$nextMonday`T080000$" -and (Ics 'Physik') -match "(?m)^DTEND;TZID=Europe/Berlin:$nextMonday`T093000$" -and (Ics 'Physik') -match '(?m)^LOCATION:Raum 204$'; text = 'Schnelleingabe: Physik jeden Montag 8 Uhr bis 9:30 Raum 204' },
            @{ ok = (Ics 'Optiker') -match "(?m)^DTSTART;TZID=Europe/Berlin:$tomorrow`T150000$" -and (Ics 'Optiker') -match '(?m)^LOCATION:Dr\. Weber$' -and (Ics 'Optiker') -match '(?m)^TRIGGER:-PT15M$'; text = 'Schnelleingabe: Optiker morgen 15 Uhr bei Dr. Weber (mit Standarderinnerung)' },
            @{ ok = ($files | Where-Object { $_.summary -eq 'Geschenk kaufen' -and $_.ics -match '(?m)^BEGIN:VTODO$' }) -and ($arbeit -contains 'Geschenk kaufen'); text = 'Schnelleingabe: Aufgabe mit #Arbeit im Kalender Arbeit' },
            @{ ok = (Ics 'Unterlagen sortieren') -match "(?m)^DUE;VALUE=DATE:$nextFriday$"; text = 'Aufgabenzeile: „bis Freitag“ wird Fälligkeit' },
            @{ ok = $bericht -and (Ics 'Gliederung') -match "(?m)^RELATED-TO;RELTYPE=PARENT:$([regex]::Escape($bericht))$"; text = 'Teilaufgabe verweist auf ihre übergeordnete Aufgabe' },
            @{ ok = ($firstRun -join "`n") -match "(?m)^palette: suche gefunden, vorschau 'Lauftreff'"; text = 'Befehlspalette: Suche findet Termin und zeigt die Vorschau' },
            @{ ok = ($firstRun -join "`n") -match "(?m)^palette: sprung nach 'Dezember \d{4}'$"; text = 'Befehlspalette: „24.12.“ springt in den Dezember' },
            @{ ok = ($firstRun -join "`n") -match '(?m)^palette: befehl ansicht 2$'; text = 'Befehlspalette: Befehl „Wochenansicht“' },
            @{ ok = (Ics 'Elternabend') -match "(?m)^DTSTART;TZID=Europe/Berlin:$($now.AddDays(2).ToString('yyyyMMdd'))T190000$" -and (Ics 'Elternabend') -match '(?m)^LOCATION:Aula$'; text = 'Befehlspalette: „Erstellen: Elternabend“ übermorgen 19 Uhr in der Aula' },
            @{ ok = ($firstRun -join "`n") -match "(?m)^journal: fremd gelesen 'Teil eins\|\|Teil zwei' tags \[`"alt`", `"fremd`"\]$"; text = 'Journal: Kalender nur für Journale wird geladen, fremder Eintrag gelesen' },
            @{ ok = $fremd -match '(?m)^BEGIN:VJOURNAL$' -and $fremd -match '(?m)^DESCRIPTION:Teil eins\\n\\nTeil zwei$' -and $fremd -match '(?m)^X-FREMD:bleibt$' -and $fremd -match "(?m)^DTSTART;TZID=Europe/Berlin:$(D 1)T213000$"; text = 'Journal: fremder Eintrag umbenannt, Rest unverändert' },
            @{ ok = $schnee -match '(?m)^BEGIN:VJOURNAL$' -and $schnee -match "(?m)^DTSTART;VALUE=DATE:$todayStamp$" -and $schnee -match '(?m)^DESCRIPTION:\*\*Kalt\*\*\\, aber schön$' -and $schnee -match '(?m)^CATEGORIES:Winter$'; text = 'Journal: neuer Eintrag mit Markdown und Tag' },
            @{ ok = ($arbeit -contains 'Erster Schnee') -and -not ($tagebuch -contains 'Erster Schnee'); text = 'Journal: in den Kalender Arbeit verschoben' },
            @{ ok = $tippen -match '(?m)^DESCRIPTION:1 2 3 4 5 6$'; text = 'Journal: schnell hintereinander gespeichert, letzter Stand auf dem Server' },
            @{ ok = -not $found.ContainsKey('Wird gelöscht') -and ($firstRun -join "`n") -match '(?m)^journal: gelöscht \(rückfrage true\)$'; text = 'Journal: Eintrag nach Rückfrage gelöscht' },
            @{ ok = ($firstRun -join "`n") -match "(?m)^palette: journal 'Erster Schnee' \(ansicht 7\)$"; text = 'Befehlspalette: Suche findet Journaleintrag' },
            @{ ok = $created.Count -eq 1; text = 'Journal: Knopf legt Journal-Kalender auf dem Server an' }
        )
        $failed = 0
        foreach ($c in $checks) {
            if ($c.ok) { Write-Host "OK   $($c.text)" } else { Write-Host "FEHL $($c.text)"; $failed++ }
        }
        if ($failed -gt 0) { throw "$failed Prüfung(en) auf dem Server fehlgeschlagen" }
        Write-Host 'Bearbeiten Ende-zu-Ende bestanden'
    }

    if ($Shots) {
        # Aus dem Cache des Testlaufs, ohne Sync (Passwort lag nur im Speicher)
        New-Item -ItemType Directory -Force $Shots | Out-Null
        $Shots = (Resolve-Path $Shots).Path
        & $Exe --size 1440x900 --snapshot "$Shots\m4-week.png"
        & $Exe --size 1440x900 --edit 'Team-Standup' --snapshot "$Shots\m4-edit-series.png"
        & $Exe --size 1440x900 --scope --snapshot "$Shots\m4-scope.png"
        & $Exe --size 1440x900 --preview 'Lauftreff' --snapshot "$Shots\m4-preview.png"
        & $Exe --size 1440x900 --view 3 --snapshot "$Shots\m4-month.png"
        & $Exe --size 1440x900 --view 6 --snapshot "$Shots\m6-tasks.png"
        & $Exe --size 1440x900 --view 4 --snapshot "$Shots\m8-year.png"
        & $Exe --size 1440x900 --view 5 --snapshot "$Shots\m8-agenda.png"
        & $Exe --size 1440x900 --palette '' --snapshot "$Shots\m8-palette.png"
        & $Exe --size 1440x900 --palette 'Lauftreff' --snapshot "$Shots\m8-palette-search.png"
        & $Exe --size 1440x900 --palette 'Kino Freitag 20 Uhr #Privat' --snapshot "$Shots\m8-palette-create.png"
        # Ohne Passwort im Speicher meldet der Sync einen Fehler: ganze Meldung
        & $Exe --size 1440x900 --view 3 --sync-popup --snapshot "$Shots\m8-sync-detail.png"
        & $Exe --size 1280x800 --view 5 --snapshot "$Shots\m8-agenda-1280.png"
        $env:HORA_THEME = 'dark'
        & $Exe --size 1440x900 --view 5 --snapshot "$Shots\m8-agenda-dark.png"
        & $Exe --size 1440x900 --view 4 --snapshot "$Shots\m8-year-dark.png"
        $env:HORA_THEME = $Theme
        & $Exe --size 1440x900 --edit-task 'Bericht schreiben' --snapshot "$Shots\m6-task-dialog.png"
        & $Exe --size 1440x900 --quick 'Mathe jeden Montag 8 Uhr bis 9:30 Raum 204' --snapshot "$Shots\m7-quick.png"
        & $Exe --size 1440x900 --quick 'todo Steuer bis 31.12. 18 Uhr #Privat' --snapshot "$Shots\m7-quick-task.png"
        & $Exe --size 1440x900 --view 7 --snapshot "$Shots\m9-journal.png"
        & $Exe --size 1440x900 --journal 'Erster Schnee' --snapshot "$Shots\m9-journal-server.png"
    }
}
finally {
    docker rm -f $name 2>$null | Out-Null
    Remove-Item Env:HORA_DATA_DIR, Env:HORA_CREDENTIALS, Env:HORA_THEME -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
