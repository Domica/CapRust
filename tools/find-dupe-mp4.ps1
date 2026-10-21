<#
.SYNOPSIS
    Pronalazi vjerovatne duplikate MP4 datoteka usporedbom po velicini (KB) i trajanju.

.DESCRIPTION
    Skenira zadani folder, za svaki .mp4 izvlaci:
      * velicinu u KB (FileInfo.Length)
      * trajanje u sekundama (ffprobe format=duration)
    Datoteke grupira po (KB, trajanje +/- tolerancija) i ispisuje grupe sa 2+ datoteke.

    Opcija -Hash dodatno racuna SHA256 za datoteke u kandidatnim grupama, pa razlikuje
    stvarni duplikat (identicni bajtovi) od samo sluzbenog para (ista velicina + trajanje,
    ali drugi encode).

.EXAMPLE
    .\find-dupe-mp4.ps1 -Path D:\Video\Raw -Recurse

.EXAMPLE
    .\find-dupe-mp4.ps1 -Path D:\Video\Raw -Recurse -Hash -ToleranceSec 0.25

.EXAMPLE
    .\find-dupe-mp4.ps1 -Path D:\Video\Raw -Recurse -Hash -Move -DryRun
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Path,

    [string]$Ffprobe = "ffprobe",

    # Minimalna velicina datoteke u KB (ignorise sitne fajlove).
    [int]$MinKB = 1,

    # Tolerancija trajanja u sekundama: 0.5 znaci "ista duzina" +/- 0.5 s.
    [double]$ToleranceSec = 0.5,

    [switch]$Recurse,

    # Ekstenzije koje se skeniraju.
    [string[]]$Extensions = @("mp4"),

    # Racuna SHA256 za kandidatne grupe (potvrda pravih duplikata).
    [switch]$Hash,

    # Premjesta duplikate (sve osim prvog) u podfolder _duplicates.
    [switch]$Move,

    # Samo ispis, bez premjestanja (skupa s -Move).
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"

# --- provjera ffprobe -------------------------------------------------------
if (-not (Get-Command $Ffprobe -ErrorAction SilentlyContinue)) {
    Write-Error "ffprobe nije na PATH-u. Proslijedi -Ffprobe 'C:\put\do\ffprobe.exe'."
    exit 1
}
$ffprobePath = (Get-Command $Ffprobe).Source

# --- prikupljanje datoteka ---------------------------------------------------
$root = (Resolve-Path -LiteralPath $Path).Path
$filter = foreach ($ext in $Extensions) {
    if ($Recurse) { Get-ChildItem -LiteralPath $root -Filter "*.$ext" -File -Recurse -ErrorAction SilentlyContinue }
    else { Get-ChildItem -LiteralPath $root -Filter "*.$ext" -File -ErrorAction SilentlyContinue }
}
$filter = @($filter | Sort-Object FullName -Unique)

$filter = $filter | Where-Object { $_.Length -ge ($MinKB * 1KB) }
if (-not $filter) {
    Write-Host "Nema .mp4 datoteka u '$root' (MinKB=$MinKB)." -ForegroundColor Yellow
    exit 0
}

Write-Host "Skeniram $($filter.Count) mp4 datoteka u '$root' ..." -ForegroundColor Cyan

# --- proba trajanja ----------------------------------------------------------
function Get-DurationSec {
    param([string]$File, [string]$Probe)
    $out = & $Probe -v error -show_entries format=duration -of csv=p=0 -- "$File" 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $out) { return $null }
    $val = ($out | Select-Object -First 1)
    if ($val -eq "N/A" -or [string]::IsNullOrWhiteSpace($val)) { return $null }
    $d = 0.0
    if (-not [double]::TryParse($val, [Globalization.NumberStyles]::Float,
            [Globalization.CultureInfo]::InvariantCulture, [ref]$d)) { return $null }
    return $d
}

$items = New-Object System.Collections.Generic.List[object]
$skipped = 0
foreach ($f in $filter) {
    $dur = Get-DurationSec -File $f.FullName -Probe $ffprobePath
    if ($null -eq $dur) { $skipped++; continue }
    $kb = [math]::Round($f.Length / 1KB)
    $bucket = [math]::Round($dur / [math]::Max($ToleranceSec, 0.001))
    $items.Add([pscustomobject]@{
        Path   = $f.FullName
        Name   = $f.Name
        Bytes  = $f.Length
        KB     = $kb
        Dur    = $dur
        Bucket = $bucket
        Key    = "$kb|$bucket"
        Hash   = $null
    })
}

Write-Host ("Procitano: {0}  (bez trajanja: {1})`n" -f $items.Count, $skipped) -ForegroundColor Cyan

# --- grupiranje --------------------------------------------------------------
$groups = @($items | Group-Object -Property Key | Where-Object { $_.Count -gt 1 } |
    Sort-Object { [double]$_.Group[0].KB })

if (-not $groups) {
    Write-Host "Nema kandidatnih grupa (ista KB + isto trajanje)." -ForegroundColor Green
    exit 0
}

# --- hash (opcionalno) -------------------------------------------------------
if ($Hash) {
    $cand = $groups | ForEach-Object { $_.Group }
    Write-Host "Hashujem $($cand.Count) datoteka (SHA256) ..." -ForegroundColor Cyan
    foreach ($it in $cand) { $it.Hash = (Get-FileHash -LiteralPath $it.Path -Algorithm SHA256).Hash }
}

function Format-Size {
    param([double]$Bytes)
    if ($Bytes -ge 1GB) { return ("{0:N2} GB" -f ($Bytes / 1GB)) }
    if ($Bytes -ge 1MB) { return ("{0:N2} MB" -f ($Bytes / 1MB)) }
    return ("{0:N0} KB" -f ($Bytes / 1KB))
}

# --- ispis -------------------------------------------------------------------
$totalWaste = 0
$groupNo = 0
foreach ($g in $groups) {
    $groupNo++
    $files = $g.Group
    $kb = $files[0].KB
    $dur = ($files | Measure-Object -Property Dur -Average).Average
    $bytes = ($files | Measure-Object -Property Bytes -Sum).Sum
    $waste = $bytes - $files[0].Bytes
    $totalWaste += $waste

    Write-Host ("`n=== Grupa {0}: {1} datoteka | {2:N0} KB | {3:N2} s | potenc. suvisno {4} ===" -f `
        $groupNo, $files.Count, $kb, $dur, (Format-Size $waste)) -ForegroundColor Yellow

    $dupHashes = @{}
    if ($Hash) {
        $dupHashes = $files | Group-Object -Property Hash | Where-Object { $_.Count -gt 1 }
    }

    $i = 0
    foreach ($it in $files | Sort-Object -Property Name) {
        $i++
        $tag = ""
        if ($Hash) {
            $isDup = ($dupHashes | Where-Object { $_.Group[0].Hash -eq $it.Hash }).Count -gt 0
            $tag = if ($isDup) { "  [DUPLIKAT]" } else { "  [samo isti KB+trajanje]" }
        }
        Write-Host ("  {0,3}. {1,-52} {2,10:N0} KB  {3,8:N3} s{4}" -f `
            $i, (Split-Path $it.Name -Leaf), $it.KB, $it.Dur, $tag) -ForegroundColor Gray
    }
}

Write-Host ("`nUkupno kandidatnih grupa: {0}   |   datoteka u grupama: {1}   |   potenc. suvisno: {2}" -f `
    $groups.Count, ($groups | ForEach-Object { $_.Count } | Measure-Object -Sum).Sum, (Format-Size $totalWaste)) -ForegroundColor Green

# --- premjestanje (opcionalno) ----------------------------------------------
if ($Move) {
    $dest = Join-Path $root "_duplicates"
    Write-Host "`nPremjestanje u '$dest' ..." -ForegroundColor Cyan
    foreach ($g in $groups) {
        $toMove = if ($Hash) {
            # premjesti samo prave duplikate (isti hash), zadrzi prvi
            @($g.Group | Group-Object -Property Hash | Where-Object { $_.Count -gt 1 } |
                ForEach-Object { $_.Group } | Sort-Object -Property Path)
        } else {
            @($g.Group | Sort-Object -Property Path)
        }

        $keep = $toMove[0]
        foreach ($it in $toMove | Select-Object -Skip 1) {
            if ($DryRun) {
                Write-Host ("  [DRY-RUN] premjesti: {0}" -f $it.Path) -ForegroundColor DarkGray
                continue
            }
            $target = Join-Path $dest $it.Name
            if (Test-Path -LiteralPath $target) {
                $base = [IO.Path]::GetFileNameWithoutExtension($it.Name)
                $ext = [IO.Path]::GetExtension($it.Name)
                $target = Join-Path $dest ("{0}_{1}{2}" -f $base, ([guid]::NewGuid().ToString("N").Substring(0,8)), $ext)
            }
            New-Item -ItemType Directory -Path $dest -Force | Out-Null
            Move-Item -LiteralPath $it.Path -Destination $target
            Write-Host ("  premjesteno: {0}  ->  {1}" -f $it.Name, $target) -ForegroundColor Green
        }
        Write-Host ("  zadrzano (original): {0}" -f $keep.Path) -ForegroundColor DarkGray
    }
}
