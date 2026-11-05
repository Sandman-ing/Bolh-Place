# Проверка установщика на чистом Windows-раннере, как это сделал бы человек:
# установить, запустить, обновить поверх запущенной программы, удалить.
# По пути делает снимки экрана в папку screens.
param([Parameter(Mandatory = $true)][string]$Installer)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
New-Item -ItemType Directory -Force screens | Out-Null
$utf8 = New-Object System.Text.UTF8Encoding $false

function Save-Screen([string]$name) {
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save("screens\$name.png", [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Host "Снимок: screens\$name.png"
}

function App-Processes { @(Get-Process bolh-place -ErrorAction SilentlyContinue) }

# Ждём только сам процесс: Start-Process -Wait ждал бы и запущенную
# установщиком программу, то есть вечно.
function Wait-Exit($p, [int]$seconds, [string]$what) {
    $null = $p.Handle  # без этого ExitCode после выхода пустой
    if (-not $p.WaitForExit($seconds * 1000)) { throw "$what не завершился за $seconds с" }
    if ($p.ExitCode -ne 0) { throw "$what вернул код $($p.ExitCode)" }
}

function Run-Installer([string[]]$arguments) {
    $p = Start-Process -FilePath $Installer -ArgumentList $arguments -PassThru
    Wait-Exit $p 180 "Установщик"
}

function Write-Settings([string]$theme) {
    $dir = Join-Path $env:APPDATA "Bolh Place"
    New-Item -ItemType Directory -Force $dir | Out-Null
    $settings = @{ spaces = @($space); active = $space; theme = $theme; language = "ru" } | ConvertTo-Json
    [IO.File]::WriteAllText((Join-Path $dir "settings.json"), $settings, $utf8)
}

# 1. Установка без вопросов.
Run-Installer @("/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART")
$appDir = Join-Path $env:LOCALAPPDATA "Programs\Bolh Place"
$exe = Join-Path $appDir "bolh-place.exe"
if (-not (Test-Path $exe)) { throw "После установки нет $exe" }
Write-Host "Установлено: $exe"

# 2. Пространство с файлами, чтобы на снимке было что смотреть.
$space = Join-Path $env:TEMP "BolhDemo\Работа"
New-Item -ItemType Directory -Force (Join-Path $space "Проекты") | Out-Null
$doc = '{"format":"bolh-doc","version":1,"id":"1b4e28ba-2fa1-11d2-883f-0016d3cca427","blocks":[{"type":"paragraph","text":"План на неделю"},{"type":"paragraph","text":"Проверить установщик"}]}'
[IO.File]::WriteAllText((Join-Path $space "План.bdoc"), $doc, $utf8)
$canvas = '{"format":"bolh-canvas","version":1,"id":"2b4e28ba-2fa1-11d2-883f-0016d3cca427","nodes":[],"edges":[]}'
[IO.File]::WriteAllText((Join-Path $space "Проекты\Схема.bcanvas"), $canvas, $utf8)
Write-Settings "dark"

# 3. Запуск установленной программы.
$first = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 12
if ($first.HasExited) { throw "Программа закрылась сама, код $($first.ExitCode)" }
Save-Screen "installed-dark"
if (-not (Test-Path (Join-Path $space ".bolh\space.json"))) { throw "Программа не открыла пространство" }

# 4. Обновление поверх запущенной программы — так его запускает автообновление.
Run-Installer @("/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/UPDATE=1")
Start-Sleep -Seconds 10
$first.Refresh()
if (-not $first.HasExited) { throw "Установщик не закрыл старую копию программы" }
$after = App-Processes
if ($after.Count -ne 1) { throw "После обновления запущено копий: $($after.Count), нужна одна" }
Save-Screen "after-update"
$after | Stop-Process -Force
Start-Sleep -Seconds 2

# 5. Светлая тема.
Write-Settings "light"
$light = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 10
if ($light.HasExited) { throw "Программа закрылась сама, код $($light.ExitCode)" }
Save-Screen "installed-light"
$light | Stop-Process -Force
Start-Sleep -Seconds 2

# 6. Удаление.
$un = Start-Process -FilePath (Join-Path $appDir "unins000.exe") -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -PassThru
Wait-Exit $un 180 "Удаление"
Start-Sleep -Seconds 3
if (Test-Path $exe) { throw "После удаления осталась программа" }
if (-not (Test-Path (Join-Path $space "План.bdoc"))) { throw "Удаление программы задело файлы пространства" }
Write-Host "Установка, запуск, обновление и удаление прошли."
