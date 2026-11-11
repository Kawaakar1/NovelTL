# Окружение для сборки NovelTL на Rust: подключите перед cargo
#   . .\env.ps1
$mingw = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Directory -Filter 'BrechtSanders.WinLibs*' |
    Select-Object -First 1 | ForEach-Object { Join-Path $_.FullName 'mingw64\bin' }
$env:Path = "$env:USERPROFILE\.cargo\bin;$mingw;$env:Path"
# короткая папка для промежуточных файлов: путь проекта очень длинный, а MinGW и Windows его не любят
$env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\noveltl-rs-target"
