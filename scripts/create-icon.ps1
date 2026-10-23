$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$image = New-Object System.Drawing.Bitmap 1024, 1024
$graphics = [System.Drawing.Graphics]::FromImage($image)
$graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$graphics.Clear([System.Drawing.Color]::Transparent)
$shape = New-Object System.Drawing.Drawing2D.GraphicsPath
$radius = 240
$shape.AddArc(0, 0, $radius, $radius, 180, 90)
$shape.AddArc((1024 - $radius), 0, $radius, $radius, 270, 90)
$shape.AddArc((1024 - $radius), (1024 - $radius), $radius, $radius, 0, 90)
$shape.AddArc(0, (1024 - $radius), $radius, $radius, 90, 90)
$shape.CloseFigure()
$darkBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml('#193d33'))
$limeBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml('#cef482'))
$graphics.FillPath($darkBrush, $shape)
$points = [System.Drawing.Point[]]@(
  [System.Drawing.Point]::new(544, 159), [System.Drawing.Point]::new(256, 576),
  [System.Drawing.Point]::new(480, 576), [System.Drawing.Point]::new(448, 865),
  [System.Drawing.Point]::new(768, 416), [System.Drawing.Point]::new(544, 416),
  [System.Drawing.Point]::new(624, 159)
)
$graphics.FillPolygon($limeBrush, $points)
$iconPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'public\app-icon.png'
$image.Save($iconPath, [System.Drawing.Imaging.ImageFormat]::Png)
$shape.Dispose(); $darkBrush.Dispose(); $limeBrush.Dispose(); $graphics.Dispose(); $image.Dispose()
Write-Host "Created $iconPath"
