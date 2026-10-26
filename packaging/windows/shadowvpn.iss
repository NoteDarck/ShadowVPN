#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#ifndef MyAppTag
  #define MyAppTag "v0.0.0"
#endif
#ifndef StageDir
  #define StageDir "..\..\ShadowVPN-stage"
#endif

[Setup]
AppId={{8F6E1C2A-6B4A-4F1C-9E2D-7D6A3B5C8E41}
AppName=ShadowVPN
AppVersion={#MyAppVersion}
AppVerName=ShadowVPN {#MyAppTag}
AppPublisher=NoteDarck
AppPublisherURL=https://github.com/NoteDarck/ShadowVPN
AppSupportURL=https://github.com/NoteDarck/ShadowVPN/issues
DefaultDirName={autopf}\ShadowVPN
DefaultGroupName=ShadowVPN
DisableProgramGroupPage=yes
PrivilegesRequired=admin
OutputDir=..\..
OutputBaseFilename=ShadowVPN-{#MyAppTag}-windows-x86_64-setup
SetupIconFile=..\..\assets\shadowvpn.ico
UninstallDisplayIcon={app}\shadowvpn-gui.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#StageDir}\shadowvpn-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\shadowvpn-client.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\shadowvpn-server.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\wintun.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\wintun-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\ShadowVPN"; Filename: "{app}\shadowvpn-gui.exe"; WorkingDir: "{app}"
Name: "{group}\Desinstalar ShadowVPN"; Filename: "{uninstallexe}"
Name: "{autodesktop}\ShadowVPN"; Filename: "{app}\shadowvpn-gui.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Registry]
Root: HKLM; Subkey: "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers"; ValueType: string; ValueName: "{app}\shadowvpn-gui.exe"; ValueData: "RUNASADMIN"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\shadowvpn-gui.exe"; Description: "{cm:LaunchProgram,ShadowVPN}"; Flags: nowait postinstall skipifsilent
