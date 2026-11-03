$recycle = Get-ChildItem 'F:\$RECYCLE.BIN' -Force -Recurse -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum
Write-Host "Recycle Bin total: $([math]::Round($recycle.Sum/1GB, 2)) GB"
Get-ChildItem 'F:\$RECYCLE.BIN' -Force -ErrorAction SilentlyContinue