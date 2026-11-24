<#
.SYNOPSIS
  Regenerates the launcher's zone art with Signet's own renderer.

.DESCRIPTION
  Reads scripts\zone-art.json (one entry per image: zone id, DAT, camera, optional spawn,
  hero and alt), renders each with
  `signet-render --screenshot`, converts it to JPEG and writes public\zones\<slug>.jpg plus
  public\zones\index.json. Runs one render at a time at BelowNormal priority.
  Re-run after renderer changes (sky, fog, lighting), then rebuild the UI and the launcher.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File ui\launcher\scripts\render-zone-art.ps1
  powershell -File ui\launcher\scripts\render-zone-art.ps1 -Only west-ronfaure -Width 2560 -Height 1440
#>
param(
    [string]$Render = "",
    [string]$Root = $env:SIGNET_FFXI_ROOT,
    [int]$Width = 1920,
    [int]$Height = 1080,
    [int]$Quality = 86,
    [string]$Only = "",
    [int]$TimeoutSec = 180
)
$ErrorActionPreference = "Stop"
if (-not $Root) { throw "pass -Root <your FFXI install folder> or set SIGNET_FFXI_ROOT" }
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$ui = Split-Path -Parent $here
$workspace = Split-Path -Parent (Split-Path -Parent $ui)
if (-not $Render) {
    foreach ($c in @("$workspace\target\release\signet-render.exe", "$workspace\target\debug\signet-render.exe")) {
        if (Test-Path $c) { $Render = $c; break }
    }
}
if (-not (Test-Path $Render)) { throw "signet-render.exe not found; build it with: cargo build --release -p signet-render" }

$out = Join-Path $ui "public\zones"
$tmp = Join-Path $env:TEMP "signet-zone-art"
New-Item -ItemType Directory -Force $out, $tmp | Out-Null
$spec = Get-Content -Raw -Encoding UTF8 (Join-Path $here "zone-art.json") | ConvertFrom-Json
Add-Type -AssemblyName System.Drawing
$codec = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object { $_.MimeType -eq "image/jpeg" }
$params = New-Object System.Drawing.Imaging.EncoderParameters 1
$params.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter ([System.Drawing.Imaging.Encoder]::Quality, [long]$Quality)

$index = @()
foreach ($z in $spec) {
    $file = "$($z.slug).jpg"
    if (-not $Only -or $Only -eq $z.slug) {
        $png = Join-Path $tmp "$($z.slug).png"
        if (Test-Path $png) { Remove-Item $png }
        $argv = @("`"$Root\$($z.dat)`"", "--root=`"$Root`"", "--screenshot=`"$png`"", "--camera=$($z.camera)", "--size=$($Width)x$($Height)")
        if ($z.actor) { $argv += "--actor=$($z.actor)"; if ($z.anim) { $argv += "--anim=$($z.anim)" } } else { $argv += "--no-actor" }
        Write-Host "render $($z.slug): $($argv -join ' ')"
        $p = Start-Process -FilePath $Render -ArgumentList $argv -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $tmp "$($z.slug).log")
        try { $p.PriorityClass = "BelowNormal" } catch {}
        if (-not $p.WaitForExit($TimeoutSec * 1000)) {
            & taskkill.exe /PID $p.Id /T /F | Out-Null
            throw "$($z.slug): render timed out"
        }
        if (-not (Test-Path $png)) { throw "$($z.slug): no image (see $tmp\$($z.slug).log)" }
        $img = [System.Drawing.Image]::FromFile($png)
        try { $img.Save((Join-Path $out $file), $codec, $params) } finally { $img.Dispose() }
        Write-Host "  wrote $file"
    }
    $entry = [ordered]@{ id = $z.id; name = $z.name; file = $file; camera = $z.camera }
    if ($z.spawn) { $entry.spawn = $z.spawn }
    # hero: the default Home image. alt: a second view of a zone (not used as that zone's art).
    if ($z.hero) { $entry.hero = $true }
    if ($z.alt) { $entry.alt = $true }
    $index += [pscustomobject]$entry
}
$json = ConvertTo-Json -InputObject $index -Depth 4
[System.IO.File]::WriteAllText((Join-Path $out "index.json"), $json, (New-Object System.Text.UTF8Encoding $false))
Write-Host "index: $($index.Count) zones"
