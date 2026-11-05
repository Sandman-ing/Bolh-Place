# Запускает программу, ждёт и снимает экран. Нужен, чтобы видеть окно в CI:
# у раннера нет монитора, но рабочий стол есть, и снимок его показывает.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$Out,
    [int]$WaitSeconds = 15
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

$proc = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Seconds $WaitSeconds
if ($proc.HasExited) {
    Write-Error "Программа закрылась сама, код $($proc.ExitCode)"
}

$bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()

Stop-Process -Id $proc.Id -Force
Write-Host "Снимок: $Out"
