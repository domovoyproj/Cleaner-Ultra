param([string]$Toolchain = '', [switch]$SkipAssets)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$manifest = Get-Content -LiteralPath Cargo.toml -Raw
if ($manifest -notmatch '(?m)^version = "(\d+\.\d+\.\d+)"') { throw 'Cargo package version not found' }
$releaseVersion = $Matches[1]
$cargoArgs = @()
if (-not $Toolchain -and (Test-Path '.photosort\tools\mingw\mingw64\bin\gcc.exe')) {
    $Toolchain = 'stable-x86_64-pc-windows-gnu'
    $env:PATH = (Join-Path (Get-Location).Path '.photosort\tools\mingw\mingw64\bin') + ';' + $env:PATH
}
if ($Toolchain) { $cargoArgs += ('+' + $Toolchain) }
foreach ($script in @('app.js','space.js','face-worker.js','icons.js','map-layout.js')) {
    & node --check (Join-Path 'photosort/web' $script)
    if ($LASTEXITCODE -ne 0) { throw "Frontend validation failed: $script" }
}
& node --test tests/map-layout.cjs
if ($LASTEXITCODE -ne 0) { throw 'Disk map layout tests failed' }
& .\packaging\prepare-runtime.ps1
& .\packaging\fetch-ffmpeg.ps1
& cargo @cargoArgs test --release --locked
if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed' }
& cargo @cargoArgs test --release --locked video_ -- --ignored
if ($LASTEXITCODE -ne 0) { throw 'Video integration tests failed' }
& cargo @cargoArgs build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
if (-not $SkipAssets) { & .\packaging\fetch-assets.ps1 }
$distRoot = [IO.Path]::GetFullPath((Join-Path (Get-Location).Path 'dist'))
$distribution = [IO.Path]::GetFullPath((Join-Path $distRoot "PhotoSort-$releaseVersion"))
if (-not $distribution.StartsWith($distRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Distribution path escaped dist'
}
if (Test-Path -LiteralPath $distRoot) {
    $distItem = Get-Item -LiteralPath $distRoot -Force
    if ($distItem.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'dist cannot be a link' }
}
if (Test-Path -LiteralPath $distribution) {
    $old = Get-Item -LiteralPath $distribution -Force
    if (-not $old.PSIsContainer -or ($old.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Distribution path is not a regular directory'
    }
    Remove-Item -LiteralPath $distribution -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $distribution | Out-Null
Copy-Item -LiteralPath target\release\photosort.exe -Destination (Join-Path $distribution 'PhotoSort.exe')
Copy-Item -LiteralPath target\release\WebView2Loader.dll -Destination $distribution
Copy-Item -LiteralPath .photosort\tools\MicrosoftEdgeWebview2Setup.exe -Destination $distribution
New-Item -ItemType Directory -Force (Join-Path $distribution 'codecs') | Out-Null
$codecFiles = Get-ChildItem -LiteralPath .photosort\tools\magick -File | Where-Object { $_.Name -eq 'magick.exe' -or $_.Extension -in '.xml','.icc' -or $_.Name -in 'LICENSE.txt','NOTICE.txt' }
foreach ($file in $codecFiles) { Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $distribution 'codecs') -Force }
New-Item -ItemType Directory -Force (Join-Path $distribution 'vision') | Out-Null
Copy-Item -Path .photosort\tools\vision\* -Destination (Join-Path $distribution 'vision') -Recurse -Force
& .\packaging\rust-licenses.ps1 -Destination (Join-Path $distribution 'licenses')
New-Item -ItemType Directory -Force (Join-Path $distribution 'ffmpeg') | Out-Null
$videoFiles = Get-ChildItem -LiteralPath .photosort\tools\ffmpeg -File | Where-Object { $_.Name -in 'ffmpeg.exe','ffprobe.exe','LICENSE.txt','COPYING.LGPLv2.1','SOURCES.txt' -or $_.Extension -eq '.dll' }
foreach ($file in $videoFiles) { Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $distribution 'ffmpeg') -Force }
$requiredFiles = @(
    'PhotoSort.exe',
    'WebView2Loader.dll',
    'MicrosoftEdgeWebview2Setup.exe',
    'codecs\magick.exe',
    'vision\vision_bundle.mjs',
    'vision\face_landmarker.task',
    'ffmpeg\ffmpeg.exe',
    'ffmpeg\ffprobe.exe',
    'ffmpeg\COPYING.LGPLv2.1'
)
foreach ($relative in $requiredFiles) {
    $candidate = Join-Path $distribution $relative
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf) -or (Get-Item -LiteralPath $candidate).Length -le 0) {
        throw "Required release file is missing or empty: $relative"
    }
}
$ffmpegLibraries = @(Get-ChildItem -LiteralPath (Join-Path $distribution 'ffmpeg') -Filter '*.dll' -File)
if ($ffmpegLibraries.Count -lt 5) { throw 'FFmpeg shared libraries are missing' }
$compiler = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'
if (-not (Test-Path $compiler)) { $compiler = Join-Path (Get-Location).Path '.photosort\tools\inno\ISCC.exe' }
if (-not (Test-Path -LiteralPath $compiler)) { throw 'Inno Setup compiler is required for a release build' }
& $compiler /Qp "/DAppVersion=$releaseVersion" packaging\installer.iss
if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
$installer = Join-Path $distRoot "PhotoSort-Setup-$releaseVersion-windows-x64.exe"
if (-not (Test-Path -LiteralPath $installer)) { throw 'Installer was not produced' }
$archive = Join-Path $distRoot "PhotoSort-$releaseVersion-windows-x64.zip"
Compress-Archive -Path (Join-Path $distribution '*') -DestinationPath $archive -Force
Get-FileHash -Algorithm SHA256 -LiteralPath $installer,$archive | ForEach-Object { $_.Hash.ToLower() + '  ' + (Split-Path $_.Path -Leaf) } | Set-Content -Encoding ascii (Join-Path $distRoot "SHA256SUMS-$releaseVersion.txt")
