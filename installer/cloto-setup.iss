; ============================================================
; ClotoCore - Windows Installer (Inno Setup)
; AI Agent Orchestration Platform
;
; Build:
;   iscc.exe cloto-setup.iss /DAppVersion=0.1.0
;
; Silent install:
;   cloto-setup-0.1.0.exe /SILENT /PORT=3000 /APIKEY=mykey
;   cloto-setup-0.1.0.exe /VERYSILENT /SUPPRESSMSGBOXES
;
; Prerequisites:
;   - Place cloto_system.exe in build/ directory
;   - icon.ico in assets/ directory (copied from dashboard/src-tauri/icons/)
; ============================================================

#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif

[Setup]
AppId={{A7E3F2B1-5C4D-4A8E-B9F0-1D2E3F4A5B6C}
AppName=ClotoCore
AppVersion={#AppVersion}
AppVerName=ClotoCore {#AppVersion}
AppPublisher=ClotoCore Project
AppPublisherURL=https://github.com/Cloto-dev/ClotoCore
AppSupportURL=https://github.com/Cloto-dev/ClotoCore/issues
AppContact=ClotoCore@proton.me
DefaultDirName={autopf}\ClotoCore
DefaultGroupName=ClotoCore
LicenseFile=..\LICENSE
OutputDir=output
OutputBaseFilename=cloto-setup-{#AppVersion}
Compression=lzma2/ultra64
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
SetupIconFile=assets\icon.ico
UninstallDisplayIcon={app}\cloto_system.exe
WizardStyle=modern
WizardImageFile=assets\wizard-image.bmp
WizardSmallImageFile=assets\wizard-small.bmp
DisableProgramGroupPage=yes
ChangesEnvironment=yes
ShowLanguageDialog=auto

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[CustomMessages]
; English
english.ComponentCore=ClotoCore System (required)
english.ComponentPython=MCP Server Support
english.ComponentDashboard=Desktop Dashboard (Tauri)
english.TaskDesktopIcon=Create a desktop shortcut
english.TaskAddToPath=Add ClotoCore to system PATH
english.TaskInstallService=Register as Windows Service (auto-start)
english.TaskGroup=Additional options:
english.ConfigPageCaption=Configuration
english.ConfigPageDescription=Configure ClotoCore server settings.
english.ConfigPortLabel=Server Port:
english.ConfigApiKeyLabel=API Key (leave empty to auto-generate):
english.ConfigGenerating=Configuring ClotoCore...
english.FinishOpenDashboard=Open ClotoCore Dashboard
; Japanese
japanese.ComponentCore=ClotoCore System (必須)
japanese.ComponentPython=MCP サーバーサポート
japanese.ComponentDashboard=デスクトップダッシュボード (Tauri)
japanese.TaskDesktopIcon=デスクトップにショートカットを作成
japanese.TaskAddToPath=ClotoCoreをシステム PATH に追加
japanese.TaskInstallService=Windows サービスとして登録 (自動起動)
japanese.TaskGroup=追加オプション:
japanese.ConfigPageCaption=設定
japanese.ConfigPageDescription=ClotoCore サーバーの設定を行います。
japanese.ConfigPortLabel=サーバーポート:
japanese.ConfigApiKeyLabel=API キー (空欄で自動生成):
japanese.ConfigGenerating=ClotoCore を構成中...
japanese.FinishOpenDashboard=ClotoCore ダッシュボードを開く

[Types]
Name: "full"; Description: "Full installation"
Name: "core"; Description: "Core only"
Name: "custom"; Description: "Custom installation"; Flags: iscustom

[Components]
Name: "core"; Description: "{cm:ComponentCore}"; Types: full core custom; Flags: fixed
Name: "python"; Description: "{cm:ComponentPython}"; Types: full custom
Name: "dashboard"; Description: "{cm:ComponentDashboard}"; Types: full custom

[Files]
; Core binary
Source: "build\cloto_system.exe"; DestDir: "{app}"; Components: core; Flags: ignoreversion
; Configuration template
Source: "..\env.example"; DestDir: "{app}"; DestName: ".env.example"; Components: core; Flags: ignoreversion
; License
Source: "..\LICENSE"; DestDir: "{app}"; Components: core; Flags: ignoreversion
; MCP scripts directory (populated at runtime via MCP server management API)
; Uninstaller helper
Source: "..\uninstall.ps1"; DestDir: "{app}"; Components: core; Flags: ignoreversion

[Icons]
Name: "{group}\ClotoCore Dashboard"; Filename: "http://localhost:{code:GetPort}"; IconFilename: "{app}\cloto_system.exe"; Comment: "Open ClotoCore Dashboard"
Name: "{group}\ClotoCore Command Line"; Filename: "{cmd}"; Parameters: "/k cd /d ""{app}"""; IconFilename: "{app}\cloto_system.exe"; Comment: "ClotoCore CLI"
Name: "{group}\Uninstall ClotoCore"; Filename: "{uninstallexe}"
Name: "{autodesktop}\ClotoCore Dashboard"; Filename: "http://localhost:{code:GetPort}"; IconFilename: "{app}\cloto_system.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "{cm:TaskDesktopIcon}"; GroupDescription: "{cm:TaskGroup}"
Name: "addtopath"; Description: "{cm:TaskAddToPath}"; GroupDescription: "{cm:TaskGroup}"; Flags: checkedonce
Name: "installservice"; Description: "{cm:TaskInstallService}"; GroupDescription: "{cm:TaskGroup}"

[Run]
; Run self-installer to set up directories and .env
Filename: "{app}\cloto_system.exe"; Parameters: "install --prefix ""{app}"" {code:GetInstallFlags}"; StatusMsg: "{cm:ConfigGenerating}"; Flags: runhidden waituntilterminated
; Optionally open dashboard after install
Filename: "http://localhost:{code:GetPort}"; Description: "{cm:FinishOpenDashboard}"; Flags: postinstall nowait shellexec skipifsilent unchecked

[UninstallRun]
; Stop service before uninstall
Filename: "sc.exe"; Parameters: "stop ClotoCore"; Flags: runhidden; RunOnceId: "StopService"
Filename: "sc.exe"; Parameters: "delete ClotoCore"; Flags: runhidden; RunOnceId: "DeleteService"

[Registry]
; Store install info for detection
Root: HKLM; Subkey: "SOFTWARE\ClotoCore"; ValueType: string; ValueName: "InstallPath"; ValueData: "{app}"; Flags: uninsdeletekey
Root: HKLM; Subkey: "SOFTWARE\ClotoCore"; ValueType: string; ValueName: "Version"; ValueData: "{#AppVersion}"; Flags: uninsdeletekey

[Code]
var
  ConfigPage: TWizardPage;
  PortEdit: TNewEdit;
  ApiKeyEdit: TNewEdit;

// --- Command-line parameter helpers ---
function GetCommandLineParam(const ParamName: String): String;
var
  I: Integer;
  Param: String;
  Prefix: String;
begin
  Result := '';
  Prefix := '/' + Uppercase(ParamName) + '=';
  for I := 1 to ParamCount do
  begin
    Param := ParamStr(I);
    if Pos(Prefix, Uppercase(Param)) = 1 then
    begin
      Result := Copy(Param, Length(Prefix) + 1, MaxInt);
      Exit;
    end;
  end;
end;

// --- Configuration page ---
procedure CreateConfigPage();
var
  PortLabel: TNewStaticText;
  ApiKeyLabel: TNewStaticText;
begin
  ConfigPage := CreateCustomPage(wpSelectTasks,
    CustomMessage('ConfigPageCaption'),
    CustomMessage('ConfigPageDescription'));

  PortLabel := TNewStaticText.Create(ConfigPage);
  PortLabel.Parent := ConfigPage.Surface;
  PortLabel.Caption := CustomMessage('ConfigPortLabel');
  PortLabel.Top := 16;
  PortLabel.Left := 0;
  PortLabel.Font.Style := [fsBold];

  PortEdit := TNewEdit.Create(ConfigPage);
  PortEdit.Parent := ConfigPage.Surface;
  PortEdit.Top := PortLabel.Top + PortLabel.Height + 4;
  PortEdit.Left := 0;
  PortEdit.Width := 120;
  // Check command-line /PORT= first
  PortEdit.Text := GetCommandLineParam('PORT');
  if PortEdit.Text = '' then
    PortEdit.Text := '8081';

  ApiKeyLabel := TNewStaticText.Create(ConfigPage);
  ApiKeyLabel.Parent := ConfigPage.Surface;
  ApiKeyLabel.Caption := CustomMessage('ConfigApiKeyLabel');
  ApiKeyLabel.Top := PortEdit.Top + PortEdit.Height + 24;
  ApiKeyLabel.Left := 0;
  ApiKeyLabel.Font.Style := [fsBold];

  ApiKeyEdit := TNewEdit.Create(ConfigPage);
  ApiKeyEdit.Parent := ConfigPage.Surface;
  ApiKeyEdit.Top := ApiKeyLabel.Top + ApiKeyLabel.Height + 4;
  ApiKeyEdit.Left := 0;
  ApiKeyEdit.Width := 400;
  // Check command-line /APIKEY= first
  ApiKeyEdit.Text := GetCommandLineParam('APIKEY');
end;

function GetPort(Param: String): String;
begin
  if PortEdit <> nil then
    Result := PortEdit.Text
  else
    Result := '8081';
  // Override from command line
  if GetCommandLineParam('PORT') <> '' then
    Result := GetCommandLineParam('PORT');
end;

// --- .env generation with user settings ---
procedure GenerateEnvFile();
var
  EnvPath: String;
  Port: String;
  ApiKey: String;
  Lines: TStringList;
begin
  EnvPath := ExpandConstant('{app}') + '\.env';
  // Don't overwrite existing .env
  if FileExists(EnvPath) then
    Exit;

  Port := GetPort('');
  ApiKey := '';
  if ApiKeyEdit <> nil then
    ApiKey := ApiKeyEdit.Text;

  // If no API key provided, let cloto_system generate one
  if ApiKey = '' then
    Exit;

  Lines := TStringList.Create;
  try
    Lines.Add('# ClotoCore Configuration (generated by installer)');
    Lines.Add('PORT=' + Port);
    Lines.Add('RUST_LOG=info');
    Lines.Add('CLOTO_API_KEY=' + ApiKey);
    Lines.Add('DATABASE_URL=sqlite:' + ExpandConstant('{app}') + '\data\cloto_memories.db');
    Lines.SaveToFile(EnvPath);
  finally
    Lines.Free;
  end;
end;

// --- Install flags ---
function GetInstallFlags(Param: String): String;
var
  Flags: String;
begin
  Flags := '';
  if WizardIsTaskSelected('installservice') then
    Flags := Flags + ' --service';
  Result := Trim(Flags);
end;

// --- PATH management ---
procedure AddToPath();
var
  CurrentPath: String;
  InstallDir: String;
begin
  InstallDir := ExpandConstant('{app}');
  if RegQueryStringValue(HKLM, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'Path', CurrentPath) then
  begin
    if Pos(Uppercase(InstallDir), Uppercase(CurrentPath)) = 0 then
    begin
      CurrentPath := CurrentPath + ';' + InstallDir;
      RegWriteStringValue(HKLM, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'Path', CurrentPath);
      Log('Added to PATH: ' + InstallDir);
    end;
  end;
end;

procedure RemoveFromPath();
var
  CurrentPath: String;
  InstallDir: String;
  NewPath: String;
  P: Integer;
begin
  InstallDir := ExpandConstant('{app}');
  if RegQueryStringValue(HKLM, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'Path', CurrentPath) then
  begin
    P := Pos(';' + Uppercase(InstallDir), Uppercase(CurrentPath));
    if P > 0 then
    begin
      NewPath := Copy(CurrentPath, 1, P - 1) + Copy(CurrentPath, P + Length(InstallDir) + 1, MaxInt);
      RegWriteStringValue(HKLM, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'Path', NewPath);
    end
    else begin
      P := Pos(Uppercase(InstallDir) + ';', Uppercase(CurrentPath));
      if P > 0 then
      begin
        NewPath := Copy(CurrentPath, 1, P - 1) + Copy(CurrentPath, P + Length(InstallDir) + 1, MaxInt);
        RegWriteStringValue(HKLM, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'Path', NewPath);
      end;
    end;
  end;
end;

// --- Lifecycle hooks ---
procedure InitializeWizard();
begin
  CreateConfigPage();
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    GenerateEnvFile();
    if WizardIsTaskSelected('addtopath') then
      AddToPath();
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath();
end;
