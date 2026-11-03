# Check Security Event Log for file deletions on F: drive
# Event ID 4663 = File access auditing (includes delete)
# Need to run as Admin for Security log

$events = Get-WinEvent -LogName Security -MaxEvents 500 -ErrorAction SilentlyContinue

$deletions = $events | Where-Object { 
    $_.Id -eq 4663 -and 
    $_.Message -like '*F:*' -and 
    ($_.Message -like '*Delete*' -or $_.Message -like '*0x10000*') 
}

if ($deletions.Count -eq 0) {
    Write-Host "Nema Event ID 4663 (brisanje) na F: drive u zadnjih 500 događaja." -ForegroundColor Yellow
    Write-Host "Provjeravam da li je File System auditing ukljucen..." -ForegroundColor Cyan
    auditpol /get /subcategory:"File System"
} else {
    $deletions | ForEach-Object {
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