; Marie LookApp Installer Script for Inno Setup
; Build with: ISCC.exe setup.iss

#define MyAppName "Marie LookApp"
#define MyAppVersion "1.0.0"
#define MyAppPublisher "lgibelli"
#define MyAppExeName "MarieLookApp.exe"

; Set architecture - pass /DArch=arm64 or /DArch=x64 when compiling
#ifndef Arch
  #define Arch "x64"
#endif

#if Arch == "arm64"
  #define MyAppArch "arm64"
  #define OutputSuffix "arm64"
#else
  #define MyAppArch "x64"
  #define OutputSuffix "x64"
#endif

[Setup]
AppId={{A1B2C3D4-E5F6-7890-ABCD-EF1234567890}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=Output
OutputBaseFilename=MarieLookApp-{#OutputSuffix}-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
#if Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "startup"; Description: "Start automatically with Windows"; GroupDescription: "Startup:"; Flags: checkedonce

[Files]
Source: "..\publish\win-{#MyAppArch}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; Startup entry
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#MyAppName}"; ValueData: """{app}\{#MyAppExeName}"""; Flags: uninsdeletevalue; Tasks: startup

; Explorer/folder background context menu
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\MarieLookApp"; ValueType: string; ValueName: ""; ValueData: "Search with Marie (Ctrl+Shift+L)"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\MarieLookApp"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#MyAppExeName}"""; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\MarieLookApp\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" --search"; Flags: uninsdeletekey

; Desktop background context menu
Root: HKCU; Subkey: "Software\Classes\DesktopBackground\shell\MarieLookApp"; ValueType: string; ValueName: ""; ValueData: "Search with Marie (Ctrl+Shift+L)"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\DesktopBackground\shell\MarieLookApp"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#MyAppExeName}"""; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\DesktopBackground\shell\MarieLookApp\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" --search"; Flags: uninsdeletekey

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
