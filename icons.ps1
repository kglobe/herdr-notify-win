# Renders Herdr's agent svg icons to 256 px png for the toast header (toasts can't show svg).
# Windows PowerShell 5.1 only (WPF). Good enough for these simple icons, not a general svg renderer:
# paths are drawn with their fill, url() fills become a gradient of the svg's colors, and an svg
# with a mask (antigravity's blurred blobs) is drawn as the mask's shape filled with that gradient.
param([string]$Icons, [string]$Out)
Add-Type -AssemblyName PresentationCore, WindowsBase

function Save-SvgPng([string]$svg, [string]$png) {
    $x = [xml](Get-Content $svg -Raw -Encoding UTF8)
    $all = { param($tag) $x.SelectNodes("//*[local-name()='$tag']") }
    $colors = @(& $all stop | ForEach-Object { $_.GetAttribute('stop-color') }) +
              @(& $all path | ForEach-Object { $_.GetAttribute('fill') } | Where-Object { $_ -match '^#' -and $_ -notmatch '^#fff' })
    $grad = [Windows.Media.LinearGradientBrush]::new()
    $grad.StartPoint = '0,0'; $grad.EndPoint = '1,1'
    for ($i = 0; $i -lt $colors.Count; $i++) {
        $grad.GradientStops.Add([Windows.Media.GradientStop]::new([Windows.Media.ColorConverter]::ConvertFromString($colors[$i]), $i / [Math]::Max(1, $colors.Count - 1)))
    }
    $mask = $x.SelectNodes("//*[local-name()='mask']")  # not via & $all: the pipeline unrolls an element into its children
    $paths = if ($mask.Count) { $mask[0].SelectNodes(".//*[local-name()='path']") } else { & $all path }
    $vb = $x.DocumentElement.GetAttribute('viewBox').Split(' ')
    $dv = [Windows.Media.DrawingVisual]::new()
    $dc = $dv.RenderOpen()
    $dc.PushTransform([Windows.Media.ScaleTransform]::new(256 / [double]$vb[2], 256 / [double]$vb[3]))
    foreach ($p in $paths) {
        $fill = $p.GetAttribute('fill')
        $brush = if ($mask.Count -or $fill -match '^url') { $grad }
                 elseif ($fill) { [Windows.Media.BrushConverter]::new().ConvertFromString($fill) }
                 else { [Windows.Media.Brushes]::Black }
        $rule = if ($p.GetAttribute('fill-rule') -eq 'evenodd') { 'F0 ' } else { 'F1 ' }
        $dc.DrawGeometry($brush, $null, [Windows.Media.Geometry]::Parse($rule + $p.GetAttribute('d')))
    }
    $dc.Close()
    $bmp = [Windows.Media.Imaging.RenderTargetBitmap]::new(256, 256, 96, 96, [Windows.Media.PixelFormats]::Pbgra32)
    $bmp.Render($dv)
    $enc = [Windows.Media.Imaging.PngBitmapEncoder]::new()
    $enc.Frames.Add([Windows.Media.Imaging.BitmapFrame]::Create($bmp))
    $fs = [IO.File]::Create($png); try { $enc.Save($fs) } finally { $fs.Close() }
}

# The toast's big logo: the agent icon with a status badge cut into its bottom-right corner.
# Hue and glyph both differ per state, so the badge reads without color too. Must match mockup/toast-mockup.html.
# Hue, lightness and glyph all differ, and none matches an agent's brand color.
$badges = @{  # color, glyph color, glyph path in a 12x12 box (same as the mockup's svg)
    ok   = '#0f7b0f', '#ffffff', 'M 2.4 6.4 L 4.8 8.7 L 9.6 3.5'
    wait = '#f0a000', '#1a1a1a', 'M 6 1.4 A 1 1 0 0 1 7 2.4 V 6.2 A 1 1 0 0 1 5 6.2 V 2.4 A 1 1 0 0 1 6 1.4 Z M 6 8.45 A 1.25 1.25 0 1 1 5.999 8.45 Z'
    init = '#687078', '#ffffff', 'M 3.8 2.2 V 9.8 L 10 6 Z'
}
function Save-Badged([string]$png, [string]$state, [string]$out) {
    $src = [Windows.Media.Imaging.BitmapImage]::new([Uri]$png)
    $d = 139; $ring = 16; $c = 256 - $d / 2 + 11          # 26/48 of the logo, 3 px gap, nudged out like the mockup
    $dv = [Windows.Media.DrawingVisual]::new(); $dc = $dv.RenderOpen()
    $cut = [Windows.Media.CombinedGeometry]::new('Exclude', [Windows.Media.RectangleGeometry]::new([Windows.Rect]::new(0, 0, 256, 256)),
        [Windows.Media.EllipseGeometry]::new([Windows.Point]::new($c, $c), $d / 2 + $ring, $d / 2 + $ring))
    $dc.PushClip($cut); $dc.DrawImage($src, [Windows.Rect]::new(0, 0, 256, 256)); $dc.Pop()
    $conv = [Windows.Media.BrushConverter]::new()
    $dc.DrawEllipse($conv.ConvertFromString($badges[$state][0]), $null, [Windows.Point]::new($c, $c), $d / 2, $d / 2)
    $g = [Windows.Media.Geometry]::Parse($badges[$state][2]).Clone()
    $k = 75 / 12                                           # 14/26 of the badge, like the mockup
    $g.Transform = [Windows.Media.MatrixTransform]::new($k, 0, 0, $k, $c - 6 * $k, $c - 6 * $k)
    $ink = $conv.ConvertFromString($badges[$state][1])
    if ($state -eq 'ok') {
        $pen = [Windows.Media.Pen]::new($ink, 2.2 * $k)
        $pen.StartLineCap = 'Round'; $pen.EndLineCap = 'Round'; $pen.LineJoin = 'Round'
        $dc.DrawGeometry($null, $pen, $g)
    } else { $dc.DrawGeometry($ink, $null, $g) }
    $dc.Close()
    $bmp = [Windows.Media.Imaging.RenderTargetBitmap]::new(256, 256, 96, 96, [Windows.Media.PixelFormats]::Pbgra32); $bmp.Render($dv)
    $enc = [Windows.Media.Imaging.PngBitmapEncoder]::new(); $enc.Frames.Add([Windows.Media.Imaging.BitmapFrame]::Create($bmp))
    $fs = [IO.File]::Create($out); try { $enc.Save($fs) } finally { $fs.Close() }
}

foreach ($n in 'claude', 'codex', 'antigravity') {
    Save-SvgPng "$Icons\$n.svg" "$Out\$n.png"
    foreach ($s in $badges.Keys) { Save-Badged "$Out\$n.png" $s "$Out\$n-$s.png" }
}
