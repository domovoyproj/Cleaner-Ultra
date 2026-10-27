#ifndef AppVersion
  #define AppVersion "0.7.0"
#endif
[Setup]
AppId={{49DBF80E-5227-44E8-B78F-206A578F593B}
AppName=Cleaner Ultra
AppVersion={#AppVersion}
AppPublisher=domovoyproj
AppPublisherURL=https://github.com/domovoyproj/PhotoSort
DefaultDirName={localappdata}\Programs\CleanerUltra
DefaultGroupName=Cleaner Ultra
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=CleanerUltra-Setup-{#AppVersion}-windows-x64
SetupIconFile=cleaner_ultra.ico
UninstallDisplayIcon={app}\CleanerUltra.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
[Files]
Source: "..\dist\CleanerUltra-{#AppVersion}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
[Icons]
Name: "{group}\CleanerUltra"; Filename: "{app}\CleanerUltra.exe"
Name: "{autodesktop}\CleanerUltra"; Filename: "{app}\CleanerUltra.exe"; Tasks: desktopicon
[Tasks]
Name: "desktopicon"; Description: "Создать ярлык на рабочем столе"; Flags: unchecked
[Run]
Filename: "{app}\MicrosoftEdgeWebview2Setup.exe"; Parameters: "/silent /install"; StatusMsg: "Установка Microsoft WebView2…"; Flags: waituntilterminated; Check: NeedsWebView2
Filename: "{app}\CleanerUltra.exe"; Description: "Запустить CleanerUltra"; Flags: nowait postinstall skipifsilent

[Code]
function NeedsWebView2: Boolean;
var
  Version: String;
  Key: String;
begin
  Key := 'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  Result := not ((RegQueryStringValue(HKCU, Key, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0')) or
    (RegQueryStringValue(HKLM32, Key, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0')));
end;
