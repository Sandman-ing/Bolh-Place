; Установщик Bolh Place (Inno Setup 6).
; Собирается скриптом scripts/build-installer.ps1, который передаёт версию.
;
; Ставится в папку пользователя, без прав администратора: тогда и
; автообновление проходит без запроса UAC.

#ifndef AppVersion
  #error Не передана версия: ISCC /DAppVersion=0.1.0
#endif

[Setup]
; AppId не менять никогда: по нему Windows узнаёт, что это обновление той же программы.
AppId={{782B64DE-B1E4-4636-973F-833EA97EE0C4}
AppName=Bolh Place
AppVersion={#AppVersion}
AppVerName=Bolh Place {#AppVersion}
AppPublisher=Bolh Place
AppPublisherURL=https://github.com/Sandman-ing/Bolh-Place
AppSupportURL=https://github.com/Sandman-ing/Bolh-Place/issues
AppUpdatesURL=https://github.com/Sandman-ing/Bolh-Place/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Bolh Place
DefaultGroupName=Bolh Place
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=BolhPlace-{#AppVersion}-setup
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Если программа открыта, установщик сам её закроет (при обновлении в том числе).
CloseApplications=force
RestartApplications=no
UninstallDisplayIcon={app}\bolh-place.exe
UninstallDisplayName=Bolh Place
ShowLanguageDialog=auto

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\target\release\bolh-place.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Bolh Place"; Filename: "{app}\bolh-place.exe"
Name: "{autodesktop}\Bolh Place"; Filename: "{app}\bolh-place.exe"; Tasks: desktopicon

[Run]
; Обычная установка: галочка «Запустить» в конце.
Filename: "{app}\bolh-place.exe"; Description: "{cm:LaunchProgram,Bolh Place}"; Flags: nowait postinstall skipifsilent
; Обновление из программы (/UPDATE=1): запустить новую версию сразу, без вопросов.
Filename: "{app}\bolh-place.exe"; Flags: nowait; Check: IsUpdate

[Code]
function IsUpdate: Boolean;
begin
  Result := ExpandConstant('{param:UPDATE|0}') = '1';
end;
