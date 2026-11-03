# Search for ANY 4663 events on F: drive
$events = Get-WinEvent -LogName Security -MaxEvents 500 -ErrorAction SilentlyContinue
$fEvents = $events | Where-Object { $_.Id -eq 4663 -and $_.Message -like '*F:*' }

if ($fEvents.Count -eq 0) {
    Write-Host "Nema NIJEDNOG Event ID 4663 na F: drive u zadnjih 500 događaja." -ForegroundColor Yellow
} else {
    Write-Host "Nađeno $($fEvents.Count) događaja na F: drive:" -ForegroundColor Green
    $fEvents | ForEach-Object {
        $msg = $_.Message
        $file = if ($msg -match 'Object Name:\s+(.+)') { $matches[1].Split("`n")[0].Trim() } else { 'N/A' }
        $proc = if ($msg -match 'Process Name:\s+(.+)') { $matches[1].Split("`n")[0].Trim() } else { 'N/A' }
        $access = if ($msg -match 'Access Mask:\s+(.+)') { $matches[1].Split("`n")[0].Trim() } else { 'N/A' }
        [PSCustomObject]@{
            Time = $_.TimeCreated
            File = $file
            Process = $proc
            AccessMask = $access
        }
    } | Format-Table -AutoSize -Wrap
}