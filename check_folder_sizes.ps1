$results = @()
$folders = Get-ChildItem F:\ -Force -Directory -ErrorAction SilentlyContinue
foreach ($folder in $folders) {
    $size = (Get-ChildItem $folder.FullName -Recurse -Force -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum
    if ($size -gt 0) {
        $results += [PSCustomObject]@{
            Name = $folder.Name
            SizeGB = [math]::Round($size/1GB, 2)
            LastWrite = $folder.LastWriteTime
        }
    }
}
$results | Sort-Object SizeGB -Descending | Format-Table -AutoSize