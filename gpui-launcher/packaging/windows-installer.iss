#define LauncherName "Breakmine Launcher"

[Setup]
AppId={{6A3F915C-8715-4614-8389-5DEFC5F992D1}
AppName={#LauncherName}
AppVersion=0.1.0
AppPublisher=Breakmine Team
DefaultDirName={localappdata}\Programs\Breakmine Launcher
DefaultGroupName={#LauncherName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist
OutputBaseFilename=Breakmine-Launcher-Windows-x64-Setup
SetupIconFile=launcher.ico
UninstallDisplayIcon={app}\launcher.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "..\..\dist\installer-input\Breakmine Launcher\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "launcher.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Breakmine Launcher"; Filename: "{app}\breakmine-launcher.exe"; WorkingDir: "{app}"; IconFilename: "{app}\launcher.ico"
Name: "{autodesktop}\Breakmine Launcher"; Filename: "{app}\breakmine-launcher.exe"; WorkingDir: "{app}"; IconFilename: "{app}\launcher.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\breakmine-launcher.exe"; Description: "Launch Breakmine Launcher"; Flags: nowait postinstall skipifsilent
