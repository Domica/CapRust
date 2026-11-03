# Quick scan for files > 1GB
$largeFiles = Get-ChildItem F:\ -Recurse -Force -ErrorAction SilentlyContinue | Where-Object { $_.Length -gt 1GB }
$largeFiles | Select-Object FullName, @{N='Size(GB)';E={'{0:N2}' -f ($_.Length/1GB)}}, LastWriteTime | Sort-Object Length -Descending | Format-Table -AutoSize