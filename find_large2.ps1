# Quick scan for files > 1GB with sizes
$largeFiles = Get-ChildItem F:\ -Recurse -Force -ErrorAction SilentlyContinue | Where-Object { $_.Length -gt 1GB }
$largeFiles | Select-Object @{N='Size(GB)';E={'{0:N2}' -f ($_.Length/1GB)}}, FullName | Sort-Object {[double]($_.'Size(GB)')} -Descending | Format-Table -AutoSize -Wrap