; BanDen installer (Inno Setup 6)
; Build from the repository root:
;   "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" scripts\installer.iss
; Output: installer\BanDen-Setup-<version>.exe

#define MyAppName "BanDen"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "BanDen contributors"
#define MyAppExeName "banden-app.exe"

[Setup]
AppId={{7C1B4E9F-2A54-4B6D-9C31-BANDENSETUP1}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE
OutputDir=..\installer
OutputBaseFilename=BanDen-Setup-{#MyAppVersion}
SetupIconFile=..\apps\desktop\src-tauri\icons\icon.ico
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
MinVersion=10.0

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\banden-app.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[Messages]
FinishedLabelNoIcons=BanDen has been installed.%n%nNote: real traffic control (device cut, per-app blocks, shaping) needs [Npcap](https://npcap.com) installed and Administrator rights. Discovery, traffic charts and all safety machinery work without it.
FinishedLabel=BanDen has been installed.%n%nNote: real traffic control (device cut, per-app blocks, shaping) needs [Npcap](https://npcap.com) installed and Administrator rights. Discovery, traffic charts and all safety machinery work without it.
