; Inno Setup script for backuptool (Windows installer)
; Build:  ISCC packaging\win\backuptool.iss   (Inno Setup: https://jrsoftware.org/isdl.php)
; Prerequisite: dist\backuptool.exe was built beforehand with build-windows.ps1.

#define MyAppName "backuptool"
#define MyAppVersion "1.2.0"
#define MyAppPublisher "Atie"
#define MyAppAuthor "Houshang Pezeshkpour"
#define MyAppContact "houshang@pezeshkpour.eu"
#define MyAppExeName "backuptool.exe"

[Setup]
AppId={{8F2C7A10-1B2C-4D3E-9F00-BACKUPTOOL01}}   ; generate your own GUID for your own builds
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppContact={#MyAppContact}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=..\..\dist
OutputBaseFilename=backuptool-setup-{#MyAppVersion}
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64
ChangesEnvironment=yes

[Languages]
Name: "german"; MessagesFile: "compiler:Languages\German.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "addtopath"; Description: "Add backuptool to the PATH (use the CLI from anywhere)"; GroupDescription: "Options:"; Flags: unchecked

[Files]
; A single portable file (built with --onefile). For --onedir, include the whole folder instead:
;   Source: "..\..\dist\backuptool\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\..\dist\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; Extend PATH (only if the task is selected)
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; \
  ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; \
  Check: NeedsAddPath('{app}'); Tasks: addtopath

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[Code]
function NeedsAddPath(Param: string): boolean;
var OrigPath: string;
begin
  if not RegQueryStringValue(HKLM,
    'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'Path', OrigPath)
  then begin Result := True; exit; end;
  Result := Pos(';' + Param + ';', ';' + OrigPath + ';') = 0;
end;
