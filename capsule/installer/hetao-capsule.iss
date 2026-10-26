; 核桃胶囊 电脑端安装程序（Inno Setup 6）
; 编译：ISCC.exe hetao-capsule.iss
; 说明：免管理员安装（装到用户目录），创建开始菜单/桌面快捷方式，可选开机自启。

#define AppName "核桃胶囊"
#define AppVer "0.1.60"
#define AppExe "hetao-capsule.exe"
#define AppPublisher "蓝胖纸"

[Setup]
AppId={{B7C3E2A1-5D4F-4A9B-9C1E-7F2A6D8B3E04}
AppName={#AppName}
AppVersion={#AppVer}
AppVerName={#AppName} {#AppVer}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=output
OutputBaseFilename=hetao-capsule-setup-{#AppVer}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ShowLanguageDialog=no
VersionInfoVersion={#AppVer}
VersionInfoCompany={#AppPublisher}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} 安装程序
; 自己结束旧进程（见 [Code]），不用 Restart Manager——托盘应用没有主窗口，RM 往往识别不到
CloseApplications=no
RestartApplications=no

[Languages]
Name: "cn"; MessagesFile: "ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "附加任务："
Name: "startup"; Description: "开机自动启动（推荐）"; GroupDescription: "附加任务："; Flags: checkedonce

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "使用说明.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#AppName}"; ValueData: """{app}\{#AppExe}"""; Flags: uninsdeletevalue; Tasks: startup

[Run]
Filename: "{app}\{#AppExe}"; Description: "立即运行 {#AppName}"; Flags: nowait postinstall skipifsilent

[Code]
/// 结束正在运行的胶囊：覆盖安装前不结束旧进程，会出现"旧版本还在跑 + 新版本又起来"两个胶囊。
/// 用 taskkill 而不是 Restart Manager：托盘应用没有可见主窗口，RM 基本识别不到。
procedure KillRunningInstances;
var
  ResultCode: Integer;
begin
  Exec('taskkill.exe', '/IM {#AppExe} /F', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  KillRunningInstances;
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  // 卸载时同样要先结束进程，否则 exe 被占用、文件删不掉
  if CurUninstallStep = usUninstall then
    KillRunningInstances;
end;
