# Find large files modified in last 5 days
$files = Get-ChildItem F:\ -Recurse -Force -ErrorAction SilentlyContinue | 
Where-Object { $_.LastWriteTime -gt (Get-Date).AddDays(-5) -and $_.Length -gt 100MB }
$files | Select-Object FullName, @{N='Size(GB)';E={'{0:N2}' -f ($_.Length/1GB)}}, LastWriteTime | Sort-Object LastWriteTime -Descending | Format-Table -AutoSize