; CutForge 安装器脚本(册六 T6.4 应用化;ADR-0022 ffmpeg 可选内嵌组件)。
;
; ── 本机构建步骤(开发机;ISCC = Inno Setup 6 编译器,https://jrsoftware.org/isinfo.php)──
;   1. cargo build --release --locked -p cutforge-render -p cutforge-cli -p cutforge-mcp
;   2. 准备 staging(与发布 zip 同一内容面,见 packaging/README.md):
;        dist-rel/bin/     ← target/release/cutforge-{cli,mcp,render}.exe
;        dist-rel/web/     ← apps/web 全树(index.html + js/ css/ assets/ + app.js/style.css 兼容别名)
;        dist-rel/scripts/ ← tools/jianying 整目录(随包剪映草稿脚本,vendor 含 pyJianYingDraft)
;   3. (可选,默认勾选的内嵌组件)把 ffmpeg.exe / ffprobe.exe 放到 packaging/ffmpeg/ ——
;      文件在位 = 组件自动启用;缺席时请在编译前删掉下方 [Files] 的 ffmpeg 组件三行。
;   4. ISCC packaging\cutforge.iss  → 产物 Output\CutForge-setup-<版本>.exe
;
; ── 诚实登记 ──
;   本脚本经结构与 Inno 语义人工核对(组件/任务/注册表/卸载面),但当前开发机无 ISCC,
;   **实际编译与纯净机安装/卸载验证为人工项**(AC-6.5,见 packaging/pure-checklist.md)。
;   `cutforge://` URL 协议注册为可选项,本版未做(见 A6-L 遗留清单)。

#define MyAppName "CutForge"
#define MyAppVersion "0.6.0"
#define MyAppPublisher "GreenChennai"
#define MyAppExeName "cutforge-cli.exe"

[Setup]
AppId={{7C1F4A2E-8B5D-4E63-9A07-3F5A1C2B4D6E}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
; .cfproj 关联写 HKCR/HKCU,安装范围按机器级;个人自用亦可降 user 级(改 PrivilegesRequired=lowest + {userpf})
PrivilegesRequired=admin
OutputBaseFilename=CutForge-setup-{#MyAppVersion}
OutputDir=Output
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; 卸载时连工程库?**不删**——工程是用户数据,卸载器只动 {app}(目录核查见 pure-checklist)
UninstallFilesDir={app}\unins

[Types]
Name: "full";    Description: "完整安装(含 ffmpeg 内嵌,推荐纯净机)"
Name: "compact"; Description: "精简安装(不自带 ffmpeg,用系统 PATH 的)"
Name: "custom";  Description: "自定义"; Flags: iscustom

[Components]
Name: "core";    Description: "CutForge 内核(三二进制 + Web 壳 + 随包脚本)"; Types: full compact custom; Flags: fixed
Name: "ffmpeg";  Description: "ffmpeg 内嵌(纯净机装完即用;ADR-0022,默认勾选)"; Types: full; Flags: checkableonce

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式(&D)"; GroupDescription: "附加图标:"
Name: "cfprojassoc"; Description: "关联 .cfproj 工程描述文件(双击打开)"; GroupDescription: "文件关联:"; Flags: checkedonce

[Files]
; ── core(固定组件)──
Source: "dist-rel\bin\cutforge-cli.exe";    DestDir: "{app}\bin";    Components: core; Flags: ignoreversion
Source: "dist-rel\bin\cutforge-mcp.exe";    DestDir: "{app}\bin";    Components: core; Flags: ignoreversion
Source: "dist-rel\bin\cutforge-render.exe"; DestDir: "{app}\bin";    Components: core; Flags: ignoreversion
Source: "dist-rel\web\*";                   DestDir: "{app}\web";    Components: core; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "dist-rel\scripts\*";               DestDir: "{app}\scripts"; Components: core; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\README.md"; DestDir: "{app}"; Components: core; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Components: core; Flags: ignoreversion
Source: "..\NOTICE.md"; DestDir: "{app}"; Components: core; Flags: ignoreversion
; ── ffmpeg 内嵌组件(构建时 packaging/ffmpeg/ 有文件即启用;ADR-0022)──
Source: "ffmpeg\ffmpeg.exe";  DestDir: "{app}\bin"; Components: ffmpeg; Flags: ignoreversion skipifsourcedoesntexist
Source: "ffmpeg\ffprobe.exe"; DestDir: "{app}\bin"; Components: ffmpeg; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\CutForge 编辑器";                Filename: "{app}\bin\{#MyAppExeName}"; Parameters: "serve --open"; Comment: "CutForge 编辑器(无参数时交互选择工程)"
Name: "{group}\CutForge 工程库";                Filename: "{app}\bin\{#MyAppExeName}"; Parameters: "library list --json"; Comment: "工程库清单(命令行)"
Name: "{group}\卸载 {#MyAppName}";              Filename: "{uninstallexe}"
Name: "{autodesktop}\CutForge 编辑器";          Filename: "{app}\bin\{#MyAppExeName}"; Parameters: "serve --open"; Tasks: desktopicon

[Registry]
; ── ffmpeg 内嵌的运行时命中方式(ADR-0022 决策 2:安装器写 env,运行时探测面零新代码)──
; 解析顺序 = env 显式(此处)→ 系统 PATH → (内嵌兜底即本 env);取消勾选组件则不写。
Root: HKCU; Subkey: "Environment"; ValueType: string; ValueName: "CUTFORGE_FFMPEG"; \
    ValueData: "{app}\bin\ffmpeg.exe";  Components: ffmpeg; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Environment"; ValueType: string; ValueName: "CUTFORGE_FFPROBE"; \
    ValueData: "{app}\bin\ffprobe.exe"; Components: ffmpeg; Flags: uninsdeletevalue
; ── .cfproj 文件关联(双击 = serve 打开;HKA 自动落在 admin=HKLM / user=HKCU)──
Root: HKA; Subkey: ".cfproj"; ValueType: string; ValueName: ""; ValueData: "CutForge.Project"; \
    Tasks: cfprojassoc; Flags: uninsdeletevalue
Root: HKA; Subkey: ".cfproj"; ValueType: string; ValueName: "Content Type"; ValueData: "application/json"; \
    Tasks: cfprojassoc; Flags: uninsdeletevalue
Root: HKA; Subkey: "CutForge.Project"; ValueType: string; ValueName: ""; ValueData: "CutForge 工程描述"; \
    Tasks: cfprojassoc; Flags: uninsdeletekey
Root: HKA; Subkey: "CutForge.Project\DefaultIcon"; ValueType: string; ValueName: ""; \
    ValueData: "{app}\bin\cutforge-cli.exe,0"; Tasks: cfprojassoc; Flags: uninsdeletekey
Root: HKA; Subkey: "CutForge.Project\shell\open\command"; ValueType: string; ValueName: ""; \
    ValueData: """{app}\bin\cutforge-cli.exe"" serve ""%1"" --open"; Tasks: cfprojassoc; Flags: uninsdeletekey
; 协议注册说明:`cutforge://` URL 协议为可选项,本版未做(A6-L)。
; 若将来启用:Root: HKA; Subkey: "Software\Classes\cutforge" + URL Protocol 值 + shell\open\command
; ValueData 含 "%1" 由首实例唤起面消费——须先做单实例唤起(第二实例把工程转交首实例),勿先注册。

[Run]
Filename: "{app}\bin\{#MyAppExeName}"; Parameters: "serve --open"; \
    Description: "启动 CutForge 编辑器"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; 随包 web/scripts 全随 {app} 卸载;.cutforge 会话记账在各工程内(用户数据,保留)。
Type: filesandordirs; Name: "{app}\web"
Type: filesandordirs; Name: "{app}\scripts"

; 已知留白(诚实登记,勿当已完成):
; · env 写入后,已开着的终端/进程看不到新值,须重开(或注销)——脚本内不做
;   WM_SETTINGCHANGE 广播(未经 ISCC 编译验证的 Pascal 代码不入库;广播留作
;   编译验证人工项的一部分,验证通过后再补)。
; · `cutforge://` 协议注册(见 [Registry] 段内注释)与单实例唤起,本版未做(A6-L)。
