# Check for hard links on large RDR2 files
$files = Get-ChildItem 'F:\Igre\RedDeadRedemption2\.egstore\bps\f' -Force -ErrorAction SilentlyContinue | Where-Object { $_.Length -gt 1GB }
foreach ($f in $files) {
    $hl = fsutil hardlink list $f.FullName 2>$null
    if ($hl) { 
        Write-Host "$($f.Name): HARD LINKS"
        $hl | ForEach-Object { Write-Host "  $_" }
    } else {
        Write-Host "$($f.Name): no hard links"
    }
}