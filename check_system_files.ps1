# Check for system files on F:
$systemFiles = @('pagefile.sys', 'hiberfil.sys', 'swapfile.sys')
foreach ($f in $systemFiles) {
    $path = "F:\$f"
    if (Test-Path $path) {
        $size = (Get-Item $path -Force).Length
        Write-Host ("{0}: {1:N2} GB" -f $f, ($size/1GB))
    } else {
        Write-Host ("{0}: not found" -f $f)
    }
}

# Check for OneDrive/Cloud folders
$cloud = Get-ChildItem F:\ -Directory -Force -ErrorAction SilentlyContinue | Where-Object { $_.Name -like '*OneDrive*' -or $_.Name -like '*Cloud*' -or $_.Name -like '*Sync*' }
$cloud | Select-Object Name, FullName | Format-Table -AutoSize