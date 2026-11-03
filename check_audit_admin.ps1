# Check audit policy and security log for F: drive deletions
# Run as Administrator!

Write-Host "=== AUDIT POLICY ===" -ForegroundColor Cyan
auditpol /get /subcategory:"File System"

Write-Host "`n=== SECURITY LOG (Event 4663 on F:) ===" -ForegroundColor Cyan
try {
    $events = Get-WinEvent -LogName Security -MaxEvents 500 -ErrorAction Stop
    $deletions = $events | Where-Object { 
        $_.Id -eq 4663 -and 
        $_.Message -like '*F:*' -and 
        ($_.Message -like '*Delete*' -or $_.Message -like '*0x10000*') 
    }
    
    if ($deletions.Count -eq 0) {
        Write-Host "Nema Event ID 4663 (brisanje) na F: drive u zadnjih 500 događaja." -ForegroundColor Yellow
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
        } | Format-Table -AutoSize -Wrap | Out-String | Write-Host
    }
} catch {
    Write-Host "Greška: $_" -ForegroundColor Red
}

Read-Host "Pritisni Enter za izlaz"