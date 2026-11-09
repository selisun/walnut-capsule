; 核桃胶囊 电脑端安装程序（Inno Setup 6）
; 编译：ISCC.exe hetao-capsule.iss
; 说明：免管理员安装（装到用户目录），创建开始菜单/桌面快捷方式，可选开机自启。

#define AppName "核桃胶囊"
#define AppVer "0.3.12"
#define AppExe "hetao-capsule.exe"
#define AppPublisher "蓝胖纸"
; 小屏进程必须随安装包一起分发，缺失时直接让编译失败，避免静默产出「没有小屏」的安装包
#define ScreenExe "..\..\windows-screen\target\release\hetao-screen.exe"
#define ScreenProc "hetao-screen.exe"
#if FileExists(ScreenExe) == 0
  #error "缺少 hetao-screen.exe：请先在 windows-screen 目录执行 cargo build --release -p screen-app"
#endif

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
; 核心安装包固定输出名：自绘安装界面 setup_ui.exe 会在同目录按这个名字调用它
OutputBaseFilename=hetao-capsule-setup-core
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
; B-10 native 引擎的小屏进程，必须与主 exe 同目录；缺失时编译期已报错，不再静默跳过
Source: "{#ScreenExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "使用说明.txt"; DestDir: "{app}"; Flags: ignoreversion
; MIT 许可全文随包落地，安装向导不再单独占一屏
Source: "LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
; 官方皮肤主题（深/白 × 横/竖 × 品牌）：写入用户主题目录
; JSON 必须随版本覆盖：新增品牌变体/修正哈希后，老用户也能拿到；
; 图片用 onlyifdoesntexist 避免覆盖用户自行替换的底图；uninsneveruninstall 卸载不删
Source: "themes\*.json"; DestDir: "{userappdata}\hetao-capsule\screen\themes"; Flags: ignoreversion
Source: "themes\*"; DestDir: "{userappdata}\hetao-capsule\screen\themes"; Flags: onlyifdoesntexist uninsneveruninstall

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
  // 主进程被强制结束后，托管的小屏进程会变成孤儿并锁住 hetao-screen.exe；
  // 不杀它，覆盖安装会报「DeleteFile 失败：错误代码 5」。
  Exec('taskkill.exe', '/IM {#ScreenProc} /F', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
end;

/// 部分机器上句柄释放有延迟，删不掉就重试几次，避免直接弹「错误代码 5」。
procedure RemoveStaleFiles;
var
  I: Integer;
  Target: String;
begin
  Target := ExpandConstant('{app}\{#ScreenProc}');
  for I := 1 to 10 do
  begin
    if not FileExists(Target) then
      Exit;
    if DeleteFile(Target) then
      Exit;
    KillRunningInstances;
    Sleep(300);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  KillRunningInstances;
  RemoveStaleFiles;
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  // 卸载时同样要先结束进程，否则 exe 被占用、文件删不掉
  if CurUninstallStep = usUninstall then
    KillRunningInstances;
end;
