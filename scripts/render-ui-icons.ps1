$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$repo = Split-Path $PSScriptRoot -Parent
$icons = Join-Path $repo 'assets\icons'
$edge = Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'
if (-not (Test-Path -LiteralPath $edge)) { throw 'Microsoft Edge not found.' }

$work = Join-Path $env:TEMP 'ut70-icon-render'
New-Item -ItemType Directory -Path $work -Force | Out-Null
foreach ($name in @('plugs', 'plugs-connected', 'gear-six', 'file-arrow-up', 'sun', 'moon')) {
    $svg = Get-Content -LiteralPath (Join-Path $icons "$name.svg") -Encoding UTF8 -Raw
    $html = '<!doctype html><html><head><meta charset="utf-8"><style>html,body{margin:0;padding:0;width:96px;height:96px;background:#fff}svg{display:block;width:96px;height:96px;color:#000}</style></head><body>' + $svg + '</body></html>'
    $page = Join-Path $work "$name.html"
    $shot = Join-Path $work "$name-screen.png"
    $profile = Join-Path $work "edge-$name"
    Set-Content -LiteralPath $page -Value $html -Encoding UTF8
    if (Test-Path -LiteralPath $shot) { Remove-Item -LiteralPath $shot }
    $url = ([Uri]$page).AbsoluteUri
    & $edge '--headless=new' '--disable-gpu' '--hide-scrollbars' '--no-first-run' "--user-data-dir=$profile" "--screenshot=$shot" '--window-size=128,128' $url | Out-Null
    for ($attempt = 0; $attempt -lt 100 -and -not (Test-Path -LiteralPath $shot); $attempt++) {
        Start-Sleep -Milliseconds 100
    }
    if (-not (Test-Path -LiteralPath $shot)) { throw "Failed to render $name.svg" }

    $source = [System.Drawing.Bitmap]::FromFile($shot)
    $output = New-Object System.Drawing.Bitmap(96, 96, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        for ($y = 0; $y -lt 96; $y++) {
            for ($x = 0; $x -lt 96; $x++) {
                $pixel = $source.GetPixel($x, $y)
                $alpha = 255 - $pixel.R
                $output.SetPixel($x, $y, [System.Drawing.Color]::FromArgb($alpha, 255, 255, 255))
            }
        }
        $output.Save((Join-Path $icons "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        $source.Dispose()
        $output.Dispose()
    }
}
