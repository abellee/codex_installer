param(
  [Parameter(Mandatory = $true)][string]$SourceRoot,
  [Parameter(Mandatory = $true)][string]$OutputRoot
)

Add-Type -AssemblyName System.Drawing

$fontFamily = New-Object System.Drawing.FontFamily("Microsoft YaHei UI")
$labelFont = New-Object System.Drawing.Font($fontFamily, 16, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
$numberFont = New-Object System.Drawing.Font($fontFamily, 15, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
$red = [System.Drawing.Color]::FromArgb(236, 61, 61)
$labelBackground = [System.Drawing.Color]::FromArgb(232, 30, 25, 25)
$white = [System.Drawing.Color]::White

function New-AnnotatedScreenshot {
  param(
    [string]$Source,
    [string]$Output,
    [array]$Annotations
  )

  $sourceImage = [System.Drawing.Image]::FromFile($Source)
  $bitmap = New-Object System.Drawing.Bitmap($sourceImage.Width, $sourceImage.Height)
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
  $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::ClearTypeGridFit
  $graphics.DrawImage($sourceImage, 0, 0, $sourceImage.Width, $sourceImage.Height)

  foreach ($annotation in $Annotations) {
    $rect = New-Object System.Drawing.Rectangle($annotation.X, $annotation.Y, $annotation.Width, $annotation.Height)
    $pen = New-Object System.Drawing.Pen($red, 4)
    $graphics.DrawRectangle($pen, $rect)

    $badge = New-Object System.Drawing.Rectangle(($annotation.X - 10), ($annotation.Y - 12), 30, 30)
    $badgeBrush = New-Object System.Drawing.SolidBrush($red)
    $graphics.FillEllipse($badgeBrush, $badge)
    $numberFormat = New-Object System.Drawing.StringFormat
    $numberFormat.Alignment = [System.Drawing.StringAlignment]::Center
    $numberFormat.LineAlignment = [System.Drawing.StringAlignment]::Center
    $badgeBounds = New-Object System.Drawing.RectangleF($badge.X, $badge.Y, $badge.Width, $badge.Height)
    $graphics.DrawString([string]$annotation.Number, $numberFont, [System.Drawing.Brushes]::White, $badgeBounds, $numberFormat)

    $pen.Dispose()
    $badgeBrush.Dispose()
    $numberFormat.Dispose()
  }

  foreach ($annotation in $Annotations) {
    $labelSize = $graphics.MeasureString($annotation.Text, $labelFont)
    $labelWidth = [Math]::Ceiling($labelSize.Width) + 24
    $labelHeight = [Math]::Ceiling($labelSize.Height) + 14
    $labelX = [Math]::Min([Math]::Max(8, $annotation.LabelX), $sourceImage.Width - $labelWidth - 8)
    $labelY = [Math]::Min([Math]::Max(8, $annotation.LabelY), $sourceImage.Height - $labelHeight - 8)
    $labelRect = New-Object System.Drawing.RectangleF($labelX, $labelY, $labelWidth, $labelHeight)
    $labelBrush = New-Object System.Drawing.SolidBrush($labelBackground)
    $graphics.FillRectangle($labelBrush, $labelRect)
    $labelPen = New-Object System.Drawing.Pen($red, 2)
    $graphics.DrawRectangle($labelPen, $labelX, $labelY, $labelWidth, $labelHeight)
    $graphics.DrawString($annotation.Text, $labelFont, [System.Drawing.Brushes]::White, ($labelX + 12), ($labelY + 7))

    $labelBrush.Dispose()
    $labelPen.Dispose()
  }

  $directory = Split-Path -Parent $Output
  New-Item -ItemType Directory -Force -Path $directory | Out-Null
  $bitmap.Save($Output, [System.Drawing.Imaging.ImageFormat]::Png)
  $graphics.Dispose()
  $bitmap.Dispose()
  $sourceImage.Dispose()
}

$slides = @(
  @{ Source="codexpp-suppliers-v4.png"; Output="codexpp-01-suppliers.png"; Marks=@(
    @{ Number=1; X=20; Y=168; Width=272; Height=44; LabelX=316; LabelY=174; Text="点击左侧「供应商配置」" }
  )},
  @{ Source="codexpp-suppliers-v5.png"; Output="codexpp-02-add.png"; Marks=@(
    @{ Number=2; X=744; Y=243; Width=112; Height=34; LabelX=690; LabelY=292; Text="点击「添加供应商」" }
  )},
  @{ Source="codexpp-access-mode.png"; Output="codexpp-03-api-mode.png"; Marks=@(
    @{ Number=3; X=346; Y=348; Width=766; Height=129; LabelX=780; LabelY=494; Text="中转站请选择「纯 API」" }
  )},
  @{ Source="codexpp-api-fields-2.png"; Output="codexpp-04-api-fields.png"; Marks=@(
    @{ Number=4; X=347; Y=348; Width=765; Height=44; LabelX=744; LabelY=402; Text="确认接入模式为「纯 API」" },
    @{ Number=5; X=347; Y=628; Width=765; Height=110; LabelX=700; LabelY=578; Text="填写 Base URL 和 API Key" }
  )},
  @{ Source="codexpp-api-fields-lower-2.png"; Output="codexpp-05-save.png"; Marks=@(
    @{ Number=6; X=348; Y=143; Width=378; Height=42; LabelX=736; LabelY=146; Text="选择 Responses API" },
    @{ Number=7; X=438; Y=541; Width=132; Height=31; LabelX=582; LabelY=541; Text="点击「从上游获取」载入模型" },
    @{ Number=8; X=347; Y=428; Width=748; Height=151; LabelX=705; LabelY=589; Text="确认模型列表和上下文窗口" },
    @{ Number=9; X=405; Y=105; Width=72; Height=32; LabelX=493; LabelY=105; Text="最后点击保存供应商" }
  )},
  @{ Source="ccswitch-main-v2.png"; Output="ccswitch-01-codex.png"; Marks=@(
    @{ Number=1; X=651; Y=44; Width=45; Height=39; LabelX=704; LabelY=48; Text="先切换到 Codex" },
    @{ Number=2; X=1104; Y=43; Width=42; Height=42; LabelX=858; LabelY=91; Text="再点击右上角加号" }
  )},
  @{ Source="ccswitch-add-screen.png"; Output="ccswitch-02-custom.png"; Marks=@(
    @{ Number=3; X=32; Y=106; Width=1118; Height=43; LabelX=62; LabelY=158; Text="保持在「Codex 供应商」" },
    @{ Number=4; X=54; Y=237; Width=177; Height=43; LabelX=245; LabelY=244; Text="选择「自定义配置」" },
    @{ Number=5; X=1054; Y=697; Width=96; Height=43; LabelX=776; LabelY=648; Text="点击添加，进入填写页面" }
  )},
  @{ Source="ccswitch-custom-fields.png"; Output="ccswitch-03-key.png"; Marks=@(
    @{ Number=6; X=53; Y=398; Width=1072; Height=70; LabelX=684; LabelY=348; Text="填写供应商名称" },
    @{ Number=7; X=54; Y=570; Width=1070; Height=76; LabelX=730; LabelY=521; Text="粘贴 API Key，切勿分享给别人" }
  )},
  @{ Source="ccswitch-custom-fields-3.png"; Output="ccswitch-04-endpoint.png"; Marks=@(
    @{ Number=8; X=54; Y=115; Width=1072; Height=91; LabelX=650; LabelY=165; Text="填写兼容 Responses API 的端点" },
    @{ Number=9; X=1083; Y=252; Width=42; Height=39; LabelX=690; LabelY=209; Text="先点击向下箭头同步上游模型" },
    @{ Number=10; X=54; Y=252; Width=1028; Height=40; LabelX=602; LabelY=318; Text="同步后点击新出现的下拉箭头，选择默认模型" },
    @{ Number=11; X=1053; Y=696; Width=98; Height=44; LabelX=794; LabelY=646; Text="确认无误后点击添加" }
  )}
)

foreach ($slide in $slides) {
  New-AnnotatedScreenshot -Source (Join-Path $SourceRoot $slide.Source) -Output (Join-Path $OutputRoot $slide.Output) -Annotations $slide.Marks
}

$labelFont.Dispose()
$numberFont.Dispose()
$fontFamily.Dispose()
