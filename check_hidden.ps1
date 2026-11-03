# Check System Volume Information
$sysVol = Get-ChildItem 'F:\System Volume Information' -Force -Recurse -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum
Write-Host "System Volume Information: $([math]::Round($sysVol.Sum/1GB, 2)) GB"

# Check for hidden/system files in root
$hidden = Get-ChildItem F:\ -Force -ErrorAction SilentlyContinue | Where-Object { $_.Attributes -band [System.IO.FileAttributes]::Hidden -or $_.Attributes -band [System.IO.FileAttributes]::System }
$hidden | Select-Object Name, @{N='Size(GB)';E={'{0:N2}' -f ($_.Length/1GB)}}, Attributes, LastWriteTime | Format-Table -AutoSize