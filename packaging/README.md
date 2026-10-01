# CutForge 打包与分发(册六 T6.4)

两个分发形态:**安装器**(Inno Setup,`cutforge.iss`)与**便携版 zip**(Release 资产,解压即用)。ffmpeg 策略见 ADR-0022:安装器可选组件(默认勾选内嵌);便携版天然内嵌;运行时解析顺序 = env 显式 → 系统 PATH → 内嵌随包(内嵌命中 = 安装器写 `CUTFORGE_FFMPEG`/`CUTFORGE_FFPROBE`)。

## 内容清单(两形态一致)

| 落点 | 内容 | 说明 |
|---|---|---|
| `bin/cutforge-cli.exe` | CLI(serve/new/library/doctor/门禁判定器) | 主入口:serve/库/诊断 |
| `bin/cutforge-mcp.exe` | MCP server(serve/stdio/run-script) | `serve` = 编辑器宿主 |
| `bin/cutforge-render.exe` | 渲染后端(ffmpeg 直出) | 导出/精确预览的子进程 |
| `bin/ffmpeg.exe` `bin/ffprobe.exe` | ffmpeg 内嵌(**可选**) | 安装器勾选组件;便携版看构建时是否打入 |
| `web/` | Web 壳全树(`index.html` + `js/ css/ assets/` + `app.js`/`style.css` 根别名) | `default_web_dir` 依次找 env → `<exe 同目录>/web` → 开发树 |
| `scripts/jianying/` | 剪映草稿随包脚本(`rs_jy_draft.py` + 依赖闭包 + vendor/pyJianYingDraft,MIT 归属) | ADR-0023:仍依赖 Python 运行时,诚实标注;`export_jianying` 定位序 = env → 工程内 → **随包** → CutFlow 回退 |
| `README.md` `LICENSE` `NOTICE.md` | 文档与许可三件套 | 安装器与便携版都带 |

> 现状注记(诚实):Release 工作流的 zip 打包步骤(`.github/workflows/gate.yml` release job)目前只拷 `web/` 三件(index.html/app.js/style.css)——`web/` 全树以安装器/本地构建的 staging 为准;release 步骤的全树化登记 A6-L 遗留。

## 安装器(推荐纯净机)

构建(开发机,四步;ISCC = [Inno Setup 6](https://jrsoftware.org/isinfo.php)):

```bat
cargo build --release --locked -p cutforge-render -p cutforge-cli -p cutforge-mcp
rem staging:dist-rel/{bin,web,scripts}(内容清单见上表)
mkdir dist-rel\bin & copy target\release\cutforge-*.exe dist-rel\bin\
robocopy apps\web dist-rel\web /E
robocopy tools\jianying dist-rel\scripts /E
rem 可选内嵌组件:把 ffmpeg.exe/ffprobe.exe 放进 packaging\ffmpeg\ 即启用(缺省勾选)
copy <你的ffmpeg>\ffmpeg.exe packaging\ffmpeg\
copy <你的ffmpeg>\ffprobe.exe packaging\ffmpeg\
ISCC packaging\cutforge.iss    rem 产物 → packaging\Output\CutForge-setup-<版本>.exe
```

安装器做掉的事:装 `{autopf}\CutForge`(core 固定 + ffmpeg 默认勾选可选)、开始菜单/桌面快捷方式(`cutforge-cli serve --open`)、**`.cfproj` 文件关联**(双击 = `cutforge-cli serve "<file>" --open`)、卸载器(用户工程与库**不删**)、勾选 ffmpeg 时写 `HKCU\Environment` 两个变量(卸载即删)。

**人工项(诚实)**:本脚本经结构人工核对,但当前开发机无 ISCC——实际编译、纯净机安装/卸载核查(注册表/目录)、`.cfproj` 双击关联生效,按 `pure-checklist.md` 在 VM 执行并登记。

## 便携版 zip

= 上表内容打包单文件 zip(`CutForge-<os>.zip` + `SHA256SUMS-<os>.txt`)。与安装器的差异:

| 维度 | 安装器 | 便携版 zip |
|---|---|---|
| ffmpeg 内嵌 | 可选组件,默认勾选 | 打包时决定(打了就有;ADR-0022「便携版天然内嵌」口径) |
| `.cfproj` 关联/快捷方式 | 安装时写注册表/建快捷方式 | 无(命令行使用;或手工执行一次关联命令) |
| `CUTFORGE_*` env | 勾选内嵌时安装器写 | 不写——用 `setx CUTFORGE_FFMPEG <zip 解压根>\bin\ffmpeg.exe` 自配,或让系统 PATH 命中 |
| 卸载 | 卸载器(目录+注册表值核查干净) | 删解压根目录即可(零注册表残留) |

启动方式(解压后):

```bat
bin\cutforge-cli serve --open      :: 无参数 = 交互选工程(回车取最近)
bin\cutforge-cli serve <工程目录|.cfproj> --open
bin\cutforge-cli doctor <工程目录> --bundle   :: 环境诊断 + 一键诊断包 zip
```

## 纯净机验收

AC-6.5/AC-6.6 的 VM 执行清单见 [pure-checklist.md](pure-checklist.md)(装机 → 全流程 → 判定;安装与真实 VM 为人工项)。
