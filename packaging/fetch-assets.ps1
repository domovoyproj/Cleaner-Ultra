$ErrorActionPreference = 'Stop'
$toolRoot = Join-Path (Split-Path $PSScriptRoot -Parent) '.photosort\tools'
New-Item -ItemType Directory -Force $toolRoot | Out-Null
$webviewSetup = Join-Path $toolRoot 'MicrosoftEdgeWebview2Setup.exe'
if (-not (Test-Path $webviewSetup)) { Invoke-WebRequest 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -OutFile $webviewSetup }
$seven = Join-Path $toolRoot '7zr.exe'
if (-not (Test-Path $seven)) { Invoke-WebRequest 'https://www.7-zip.org/a/7zr.exe' -OutFile $seven }
$codecRoot = Join-Path $toolRoot 'magick'
if (-not (Test-Path (Join-Path $codecRoot 'magick.exe'))) {
    $archive = Join-Path $toolRoot 'imagemagick.7z'
    Invoke-WebRequest 'https://github.com/ImageMagick/ImageMagick/releases/download/7.1.2-32/ImageMagick-7.1.2-32-portable-Q8-x64.7z' -OutFile $archive
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLower() -ne '1fdfe4558dcf17a72ff5f721b987a86949e1bcde3d53a70ce2b39c84f8815325') {
        throw 'ImageMagick archive checksum mismatch'
    }
    & $seven x -y ('-o' + $codecRoot) $archive | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Codec extraction failed' }
}
$visionRoot = Join-Path $toolRoot 'vision'
New-Item -ItemType Directory -Force $visionRoot | Out-Null
if (-not (Test-Path (Join-Path $visionRoot 'vision_bundle.mjs'))) {
    $package = Join-Path $toolRoot 'vision.tgz'
    Invoke-WebRequest 'https://registry.npmjs.org/@mediapipe/tasks-vision/-/tasks-vision-1.1.0.tgz' -OutFile $package
    tar -xzf $package -C $visionRoot --strip-components=1
    if ($LASTEXITCODE -ne 0) { throw 'Vision extraction failed' }
}
$model = Join-Path $visionRoot 'face_landmarker.task'
if (-not (Test-Path $model)) { Invoke-WebRequest 'https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task' -OutFile $model }
Invoke-WebRequest 'https://raw.githubusercontent.com/google-ai-edge/mediapipe/master/LICENSE' -OutFile (Join-Path $visionRoot 'LICENSE.txt')
