# Check SACL on a specific folder user owns
$folder = "F:\Dev\projects\CapRust"
if (Test-Path $folder) {
    try {
        $acl = Get-Acl $folder -Audit
        Write-Host ("SACL for {0}:" -f $folder) -ForegroundColor Cyan
        $acl.Audit | ForEach-Object {
            [PSCustomObject]@{
                Identity = $_.IdentityReference
                FileSystemRights = $_.FileSystemRights
                AccessControlType = $_.AccessControlType
                InheritanceFlags = $_.InheritanceFlags
                PropagationFlags = $_.PropagationFlags
            }
        } | Format-Table -AutoSize
    } catch {
        Write-Host ("Error (need admin for SACL): {0}" -f $_) -ForegroundColor Red
    }
} else {
    Write-Host ("Folder not found: {0}" -f $folder) -ForegroundColor Red
}