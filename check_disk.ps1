$dirs = Get-ChildItem C:\ -Directory -ErrorAction SilentlyContinue
$results = @()
foreach ($d in $dirs) {
    $size = (Get-ChildItem $d.FullName -Recurse -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum
    $results += [PSCustomObject]@{Name=$d.Name; SizeGB=[math]::Round($size/1GB,2)}
}
$results | Sort-Object SizeGB -Descending | Select-Object -First 20 | Format-Table -AutoSize