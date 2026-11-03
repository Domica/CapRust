# Test audit logging
New-Item F:\test_audit_delete.txt -Force
Remove-Item F:\test_audit_delete.txt -Force

$events = Get-WinEvent -LogName Security -MaxEvents 10 -ErrorAction SilentlyContinue
$testEvents = $events | Where-Object { $_.Id -eq 4663 -and $_.Message -like '*test_audit*' }

if ($testEvents.Count -eq 0) {
    Write-Host "NIJE LOGIRANO - auditing ne radi ili log prekrčen" -ForegroundColor Red
} else {
    $testEvents | ForEach-Object {
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
    } | Format-Table -AutoSize
}