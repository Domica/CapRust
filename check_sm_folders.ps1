# Check all StabilityMatrix folders sizes
$folders = Get-ChildItem F:\StabilityMatrix* -Force -Directory -ErrorAction SilentlyContinue
$results = @()
foreach ($f in $folders) {
    $size = (Get-ChildItem $f.FullName -Recurse -Force -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum
    $results += [PSCustomObject]@{
        Name = $f.Name
        SizeGB = [math]::Round($size/1GB, 2)
        LastWrite = $f.LastWriteTime
    }
}
$results | Format-Table -AutoSize