$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$manifest = Get-Content (Join-Path $PSScriptRoot 'ffmpeg.json') -Raw | ConvertFrom-Json
$toolRoot = Join-Path $projectRoot '.cleaner_ultra\tools'
New-Item -ItemType Directory -Force $toolRoot | Out-Null
$destination = Join-Path $toolRoot 'ffmpeg'
if (Test-Path (Join-Path $destination 'ffmpeg.exe')) { return }
$archive = Join-Path $toolRoot 'ffmpeg-lgpl.zip'
$downloaded = $false
foreach ($sourceUrl in @($manifest.url, $manifest.mirror) | Where-Object { $_ }) {
    try {
        Invoke-WebRequest $sourceUrl -OutFile $archive
        if ((Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLower() -ne $manifest.sha256) { throw 'FFmpeg archive checksum mismatch' }
        $downloaded = $true
        break
    } catch { Write-Warning "FFmpeg source unavailable or invalid: $sourceUrl" }
}
if (-not $downloaded) { throw 'No verified FFmpeg archive available' }
if ((Get-FileHash -Algorithm SHA256 $archive).Hash.ToLower() -ne $manifest.sha256) { throw 'FFmpeg archive checksum mismatch' }
$expanded = Join-Path $toolRoot 'ffmpeg-expanded'
Expand-Archive -LiteralPath $archive -DestinationPath $expanded -Force
$binary = Get-ChildItem -LiteralPath $expanded -Filter ffmpeg.exe -Recurse | Select-Object -First 1
if (-not $binary) { throw 'FFmpeg executable missing from archive' }
$package = Split-Path (Split-Path $binary.FullName -Parent) -Parent
New-Item -ItemType Directory -Force $destination | Out-Null
Copy-Item -Path (Join-Path (Split-Path $binary.FullName -Parent) '*') -Destination $destination -Force
Get-ChildItem -LiteralPath $package -File | Where-Object Name -Match '^(LICENSE|COPYING|NOTICE)' | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $destination }
"FFmpeg LGPL shared build. Original binaries: $($manifest.url)`nSHA-256: $($manifest.sha256)`nCorresponding FFmpeg source: $($manifest.source)`nBuild recipes and dependency sources: $($manifest.build)`nFFmpeg license information: https://ffmpeg.org/legal.html`nThe FFmpeg executables and DLLs can be replaced independently of CleanerUltra." | Set-Content -Encoding utf8 (Join-Path $destination 'SOURCES.txt')
Invoke-WebRequest 'https://raw.githubusercontent.com/FFmpeg/FFmpeg/92611900bb/COPYING.LGPLv2.1' -OutFile (Join-Path $destination 'COPYING.LGPLv2.1')
