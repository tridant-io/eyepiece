; the windows installer (adr 0008): everything scripts\package.ps1 puts in dist\eyepiece, installed for the user alone
; in %LOCALAPPDATA%\Programs\eyepiece, with a start menu entry and an uninstaller in settings > apps. it is the update
; too: the player starts it with /SILENT /NORESTART /update=<its pid> (src\update\win.rs), and it waits for that player
; to end, installs, and starts the new build on the files the player had open, which come as EYEPIECE_REOPEN.
; scripts\package.ps1 compiles it into dist\eyepiece-<version>-windows-setup.exe:
;   iscc /DVersion=<x.y.z> [/DSigned /Sartifact=<signtool command>] scripts\installer.iss

#ifndef Version
  #error pass the version from Cargo.toml: /DVersion=x.y.z
#endif

[Setup]
AppId={{CAA41D42-031D-4C63-9428-9A256052BDB2}
AppName=eyepiece
AppVersion={#Version}
AppPublisher=Tridant, Inc.
VersionInfoVersion={#Version}
; for this user alone, where installs were before there was an installer: no admin prompt
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\eyepiece
; no page for the folder or a start menu group. inno shows the ready page whenever no page comes before it, so
; that is the one click
DisableDirPage=yes
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=..\assets\icon\eyepiece.ico
UninstallDisplayIcon={app}\eyepiece.exe
WizardStyle=modern
OutputDir=..\dist
OutputBaseFilename=eyepiece-{#Version}-windows-setup
; the shell is told when [Registry]'s open with comes and goes
ChangesAssociations=yes
#ifdef Signed
SignTool=artifact
SignedUninstaller=yes
#endif

[Files]
; ignoreversion: a library of the same version, signed again, still goes in
Source: "..\dist\eyepiece\*"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\eyepiece"; Filename: "{app}\eyepiece.exe"

; open with: explorer's "open with" and settings > apps > default apps offer eyepiece for the videos it plays, and it is
; the default for none of them. windows lets only the user choose a default, so nothing here writes one: no
; UserChoice, no extension's own value. the uninstaller takes away what is written here and nothing else, and a key
; that may hold other programs' values only once it is empty
[Registry]
; the type eyepiece opens a video as. with no name of its own, explorer names each kind as it does now
Root: HKCU; Subkey: "Software\Classes\eyepiece.video"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\eyepiece.video\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\eyepiece.exe,0"
Root: HKCU; Subkey: "Software\Classes\eyepiece.video\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\eyepiece.exe"" ""%1"""
; the program as "open with" names it: the exe carries no name of its own
Root: HKCU; Subkey: "Software\Classes\Applications\eyepiece.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "eyepiece"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Applications\eyepiece.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\eyepiece.exe"" ""%1"""
; default apps' page for eyepiece, which its settings open by the name "eyepiece" (src\ui\panel.rs)
Root: HKCU; Subkey: "Software\Tridant"; Flags: uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Tridant\eyepiece"; Flags: uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Tridant\eyepiece\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "eyepiece"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Tridant\eyepiece\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "a video review player for video professionals"
Root: HKCU; Subkey: "Software\Tridant\eyepiece\Capabilities"; ValueType: string; ValueName: "ApplicationIcon"; ValueData: "{app}\eyepiece.exe,0"
Root: HKCU; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "eyepiece"; ValueData: "Software\Tridant\eyepiece\Capabilities"; Flags: uninsdeletevalue
; each video it plays: src\browse's clips, less mxf and avi, which do not play yet
#dim Videos[5] {".mov", ".mp4", ".m4v", ".mkv", ".webm"}
#define I
#sub Video
Root: HKCU; Subkey: "Software\Classes\{#Videos[I]}"; Flags: uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\{#Videos[I]}\OpenWithProgids"; ValueType: string; ValueName: "eyepiece.video"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\eyepiece.exe\SupportedTypes"; ValueType: string; ValueName: "{#Videos[I]}"; ValueData: ""
Root: HKCU; Subkey: "Software\Tridant\eyepiece\Capabilities\FileAssociations"; ValueType: string; ValueName: "{#Videos[I]}"; ValueData: "eyepiece.video"
#endsub
#for {I = 0; I < DimOf(Videos); I++} Video

[Run]
Filename: "{app}\eyepiece.exe"; Description: "{cm:LaunchProgram,eyepiece}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\eyepiece.exe"; Parameters: "{code:Reopened}"; Flags: nowait; Check: Updating

[Code]
const
  SYNCHRONIZE = $00100000;

function OpenProcess(Access: Cardinal; Inherit: Boolean; Pid: Cardinal): THandle;
  external 'OpenProcess@kernel32.dll stdcall';
function WaitForSingleObject(Handle: THandle; Milliseconds: Cardinal): Cardinal;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function CloseHandle(Handle: THandle): Boolean;
  external 'CloseHandle@kernel32.dll stdcall';

function Updating: Boolean;
begin
  Result := ExpandConstant('{param:update}') <> '';
end;

// the player that started the update holds its files and the one-window name until it has ended, so the new build
// is started only after. not waited for forever: one still running after that is closed by inno, as a silent install
// closes what holds its files
function InitializeSetup: Boolean;
var
  Player: THandle;
begin
  if Updating then begin
    Player := OpenProcess(SYNCHRONIZE, False, StrToIntDef(ExpandConstant('{param:update}'), 0));
    if Player <> 0 then begin
      WaitForSingleObject(Player, 30000);
      CloseHandle(Player);
    end;
  end;
  Result := True;
end;

// the files the player had open, as the new build's command line, quoted by the player
function Reopened(Param: String): String;
begin
  Result := GetEnv('EYEPIECE_REOPEN');
end;
