$ErrorActionPreference = 'SilentlyContinue'

Write-Host '=== Edge procesi ==='
Get-Process msedge -ErrorAction SilentlyContinue | Select-Object Name, ID, @{N='PM(MB)';E={[math]::Round($_.PM / 1MB, 2)}} | Format-Table -AutoSize

Write-Host ''
Write-Host '=== LOCALAPPDATA Edge User Data ==='
$base1 = "$env:LOCALAPPDATA\Microsoft\Edge\User Data\"
if (Test-Path $base1) {
    Get-ChildItem $base1 | Where-Object { $_.PSIsContainer } | ForEach-Object {
        $size = (Get-ChildItem $_.FullName -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
        [PSCustomObject]@{
            Folder = $_.Name
            SizeMB = [math]::Round($size / 1MB, 2)
            Path = $_.FullName
        }
    } | Sort-Object SizeMB -Descending | Format-Table -AutoSize -Wrap
} else {
    Write-Host 'Nije pronađeno'
}

Write-Host ''
Write-Host '=== APPDATA Edge User Data ==='
$base2 = "$env:APPDATA\Microsoft\Edge\User Data\"
if (Test-Path $base2) {
    Get-ChildItem $base2 | Where-Object { $_.PSIsContainer } | ForEach-Object {
        $size = (Get-ChildItem $_.FullName -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
        [PSCustomObject]@{
            Folder = $_.Name
            SizeMB = [math]::Round($size / 1MB, 2)
            Path = $_.FullName
        }
    } | Sort-Object SizeMB -Descending | Format-Table -AutoSize -Wrap
} else {
    Write-Host 'Nije pronađeno'
}
