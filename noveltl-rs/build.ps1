# Сборка NovelTL на Rust: программа + папка для раздачи + установщик.
#   powershell -ExecutionPolicy Bypass -File build.ps1 [-NoInstaller]
param([switch]$NoInstaller)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent $root            # папка Python-версии: там files\, ИНСТРУКЦИЯ.txt, installer\
Set-Location $root
. "$root\env.ps1"

$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version\s*=\s*"(.+?)"').Matches[0].Groups[1].Value

cargo test -p noveltl-core --quiet
if ($LASTEXITCODE) { throw 'тесты ядра не прошли' }
cargo build --release -p noveltl
if ($LASTEXITCODE) { throw 'сборка не удалась' }

$dist = "$root\dist\NovelTL"
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item "$env:CARGO_TARGET_DIR\release\noveltl.exe" "$dist\NovelTL.exe"
Copy-Item "$repo\files" "$dist\files" -Recurse
Copy-Item "$repo\ИНСТРУКЦИЯ.txt" $dist

# самопроверка: движок LunaHook грузится из собранной папки
$p = Start-Process "$dist\NovelTL.exe" -ArgumentList '--selfcheck' -PassThru -Wait
if ($p.ExitCode -ne 0) { throw "самопроверка не прошла (код $($p.ExitCode)), см. $dist\data\selfcheck.json" }
Remove-Item "$dist\data" -Recurse -Force -ErrorAction SilentlyContinue
$mb = [math]::Round((Get-ChildItem $dist -Recurse -File | Measure-Object Length -Sum).Sum / 1MB, 1)
Write-Host "Готово: $dist\NovelTL.exe ($mb МБ вместе с LunaHook), версия $version"

if ($NoInstaller) { return }
$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { Write-Host 'Inno Setup не найден — установщик не собран'; return }
# Inno Setup не любит длинные пути: собираем через короткую ссылку на папку проекта
$link = Join-Path $env:TEMP "ntlrs$PID"
cmd /c mklink /J "$link" "$repo" | Out-Null
try {
    & $iscc "/DAppVersion=$version" "/DSourceDir=..\noveltl-rs\dist\NovelTL" "$link\installer\NovelTL.iss"
    if ($LASTEXITCODE) { throw 'установщик не собран' }
} finally {
    cmd /c rmdir "$link" | Out-Null   # удаляет только ссылку
}
Write-Host "Установщик: $repo\dist_installer\NovelTL-Setup-$version.exe"
