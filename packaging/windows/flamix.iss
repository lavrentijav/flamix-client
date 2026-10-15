; Inno Setup script for Flamix (Windows x86_64).
;
; Per-user install by default (no UAC); the user may opt into an all-users
; install from the dialog. Build from CI with:
;
;   iscc /DAppVersion=0.1.0 ^
;        /DSourceDir=C:\path\to\staging ^
;        /DOutputDir=C:\path\to\dist ^
;        /DIconFile=C:\path\to\flamix.ico ^
;        packaging\windows\flamix.iss

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "."
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif
#ifndef IconFile
  #define IconFile "flamix.ico"
#endif

#define AppName "Flamix"
#define AppPublisher "Flamix"
#define AppURL "https://github.com/lavrentijav/flamix-client"
#define AppExeName "flamix.exe"

[Setup]
AppId={{8F3B2C6E-9A54-4D2B-9C7E-0F1A2B3C4D5E}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases
DefaultDirName={autopf}\Flamix
DefaultGroupName=Flamix
DisableProgramGroupPage=yes
LicenseFile={#SourceDir}\README.txt
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir={#OutputDir}
OutputBaseFilename=flamix-{#AppVersion}-windows-x64-setup
SetupIconFile={#IconFile}
UninstallDisplayIcon={app}\{#AppExeName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\flamix.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\cores\*"; DestDir: "{app}\cores"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#IconFile}"; DestDir: "{app}"; DestName: "flamix.ico"; Flags: ignoreversion
Source: "{#SourceDir}\README.txt"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{group}\Flamix"; Filename: "{app}\{#AppExeName}"
Name: "{group}\{cm:UninstallProgram,Flamix}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Flamix"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Registry]
; Register the flamix:// URL protocol for the current user.
Root: HKCU; Subkey: "Software\Classes\flamix"; ValueType: string; ValueName: ""; ValueData: "URL:Flamix protocol"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\flamix"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\flamix\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\flamix.ico"
Root: HKCU; Subkey: "Software\Classes\flamix\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""; Flags: uninsdeletekey

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram,Flamix}"; Flags: nowait postinstall skipifsilent
