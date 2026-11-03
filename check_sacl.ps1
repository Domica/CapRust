# Check SACL (audit entries) on F: drive root
$acl = Get-Acl F:\ -Audit
$acl.Audit | ForEach-Object {
    [PSCustomObject]@{
        Identity = $_.IdentityReference
        FileSystemRights = $_.FileSystemRights
        AccessControlType = $_.AccessControlType
        InheritanceFlags = $_.InheritanceFlags
        PropagationFlags = $_.PropagationFlags
    }
} | Format-Table -AutoSize