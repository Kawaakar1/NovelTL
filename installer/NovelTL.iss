; Сценарий установщика NovelTL (Inno Setup 6).
; Версия передаётся снаружи: ISCC /DAppVersion=0.2.0 NovelTL.iss
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
; Откуда брать программу (build.ps1 передаёт /DSourceDir сам)
#ifndef SourceDir
  #define SourceDir "..\noveltl-rs\dist\NovelTL"
#endif

[Setup]
; Фиксированный GUID приложения (не менять, иначе обновления станут отдельной программой)
AppId={{14E2CE36-5E0B-4069-88D6-CC58278DC280}
AppName=NovelTL
AppVersion={#AppVersion}
AppVerName=NovelTL {#AppVersion}
AppPublisher=NovelTL
; Папка внутри профиля пользователя: программа пишет данные рядом с exe
DefaultDirName={userpf}\NovelTL
DisableProgramGroupPage=yes
DefaultGroupName=NovelTL
; Без прав администратора
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist_installer
OutputBaseFilename=NovelTL-Setup-{#AppVersion}
; значок из репозитория (папка build\ временная и в git не хранится)
SetupIconFile=..\noveltl-rs\noveltl\assets\icon.ico
UninstallDisplayIcon={app}\NovelTL.exe
UninstallDisplayName=NovelTL
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Не ставить поверх запущенной программы
AppMutex=NovelTL_Running
CloseApplications=no
DisableWelcomePage=no

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "Создать значок на рабочем столе"
Name: "autostart"; Description: "Запускать NovelTL вместе с Windows (программа сама подхватит игру)"; Flags: unchecked

[Files]
; Тестовые данные (data) и скриншоты в установщик не входят
Source: "{#SourceDir}\*"; DestDir: "{app}"; Excludes: "data\*,data,shots_exe\*,shots_exe"; Flags: ignoreversion recursesubdirs createallsubdirs

[InstallDelete]
; при обновлении убрать файлы прошлой сборки (например, _internal Python-версии при переходе на Rust)
Type: filesandordirs; Name: "{app}\_internal"

[Icons]
Name: "{autoprograms}\NovelTL"; Filename: "{app}\NovelTL.exe"
Name: "{autoprograms}\NovelTL — демо-режим"; Filename: "{app}\NovelTL.exe"; Parameters: "--demo"
Name: "{autoprograms}\Инструкция NovelTL"; Filename: "{app}\ИНСТРУКЦИЯ.txt"
Name: "{autodesktop}\NovelTL"; Filename: "{app}\NovelTL.exe"; Tasks: desktopicon

[Registry]
; Автозапуск вместе с Windows (только для текущего пользователя)
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "NovelTL"; ValueData: """{app}\NovelTL.exe"""; Tasks: autostart; Flags: uninsdeletevalue

[Run]
Filename: "{app}\NovelTL.exe"; Description: "Запустить NovelTL"; Flags: nowait postinstall skipifsilent

[Code]
// Подсказка про Защитник Windows на последней странице
procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpFinished then
    WizardForm.FinishedLabel.Caption := WizardForm.FinishedLabel.Caption + #13#10#13#10 +
      'Если Защитник Windows удалит программу или игра не отвечает при подключении, добавьте папку ' + ExpandConstant('{app}') + ' в исключения Защитника (Безопасность Windows → Защита от вирусов и угроз → Управление настройками → Исключения).';
end;

// При удалении предлагаем стереть данные пользователя (в тихом режиме не трогаем)
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    if (not UninstallSilent) and DirExists(ExpandConstant('{app}\data')) then
    begin
      if MsgBox('Удалить также настройки, профили игр и кэш переводов?' + #13#10 + 'Если вы ещё будете пользоваться NovelTL, нажмите «Нет».', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      begin
        DelTree(ExpandConstant('{app}\data'), True, True, True);
        RemoveDir(ExpandConstant('{app}'));
      end;
    end;
  end;
end;
