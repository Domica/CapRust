$ErrorActionPreference = 'SilentlyContinue'

$base = "$env:LOCALAPPDATA\Microsoft\Edge\User Data\Default"
if (-not (Test-Path $base)) {
    Write-Host "Default folder nije pronađen: $base"
    exit
}

Write-Host "=== Veličina pod-foldera u Default (ukupno: $base) ==="
Get-ChildItem $base | Where-Object { $_.PSIsContainer } | ForEach-Object {
    $size = (Get-ChildItem $_.FullName -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
    [PSCustomObject]@{
        Folder = $_.Name
        SizeMB = [math]::Round($size / 1MB, 2)
        Path = $_.FullName
    }
} | Sort-Object SizeMB -Descending | Format-Table -AutoSize -Wrap
