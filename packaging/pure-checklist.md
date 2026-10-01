# CutForge 纯净机验收清单(AC-6.5 / AC-6.6)

> 计划书 06 册六 T6.5:**纯净验收机 = 全新 Windows 虚拟机/沙箱**——无 Python、无 Node、
> 无外部管线仓库、无 ffmpeg(装 ffmpeg 内嵌组件时)。本清单是脚本化验收的**人工执行面**:
> 装机/VM 操作无法在本仓库自动化,逐项打勾后把结果存档到本目录(`pure-checklist-run-<日期>.md`,
> AC-6.6「验收机报告存档」)。
>
> 状态(诚实登记):**未执行**——册六收口(F4)只落清单与脚本面;真实 VM 执行为人工项。

## 0. 准备

- [ ] VM 快照回滚点:全新 Windows 10/11,无开发工具链
- [ ] 核查命令(应全部「未找到」):`python --version` / `node --version` / `ffmpeg -version` / `git --version`
- [ ] 检查体:取 `packaging/Output/CutForge-setup-<版本>.exe`(构建步骤见 [README.md](README.md))或便携版 zip

## 1. 安装器路径(AC-6.5)

- [ ] 双击安装器 → 完整安装(ffmpeg 内嵌默认勾选)
- [ ] 目录核查:`<安装目录>\bin`(3+2 二进制)、`web\`(全树含 js/)、`scripts\jianying\`、许可三件套
- [ ] 注册表核查:
  - [ ] `HKCU\Environment\CUTFORGE_FFMPEG` / `CUTFORGE_FFPROBE` = `<安装目录>\bin\*.exe`
  - [ ] `HKCR\.cfproj` → `CutForge.Project` → `shell\open\command` = `"...cutforge-cli.exe" serve "%1" --open`
- [ ] 快捷方式核查:开始菜单(编辑器/工程库/卸载器)+ 桌面(勾了任务才有)
- [ ] **新开**终端 `set CUTFORGE` 应见两个 env 值(已开的旧终端看不到属正常,登记)

## 2. 全流程(AC-6.6,零 Python/零外部管线)

- [ ] 桌面快捷方式启动 → 浏览器自动打开(或控制台 URL;**端口占用时横幅提示自动换端口**)
- [ ] 新建工程:向导三模板任选(画幅/轨道预填)→ 布局 v2(或 v3)
- [ ] 导入素材:手机竖屏视频一份 + 夹具媒体;时长自动探测
- [ ] 剪辑:分割/拖拽/变速任一组;字幕(SRT 导入或新建文本 + 花字模板);音频(卡点吸附或 BGM)
- [ ] 导出:导出矩阵任一格式(缺省 mp4-h264)→ 进度 → 完成 toast;编辑器全程不阻塞
- [ ] **进程核查(AC-6.3 纯净机面)**:任务管理器确认 CutForge 子树只有 cutforge-render/ffmpeg,
       无 python(剪映草稿导出不点 = 豁免不触发)
- [ ] 产物核查:ffprobe 不在纯净机——用播放器目测时长 + 双击产物可播;如带 ffprobe 则时长对拍
- [ ] 诊断包:菜单/命令 `cutforge-cli doctor <工程> --bundle` → zip 可解压,含 doctor.json/environment.txt

## 3. 文件关联与单实例(AC-6.5)

- [ ] 工程库新建第二工程 → `cutforge-cli library cfproj <名>` 导出 `.cfproj` → 双击 = 浏览器打开该工程
- [ ] `.cfproj` 指向的工程目录改名后再双击 → 应给「工程不可识别」的可读错误(不静默)
- [ ] 同端口第二次启动 → 横幅「已自动改用 <N+1>」(单实例唤起协议 `cutforge://` 为可选未做,登记)

## 4. 卸载干净(AC-6.5)

- [ ] 控制面板卸载 → 目录核查:`<安装目录>` 仅剩 `unins\`(或全删);**用户工程库原样保留**
- [ ] 注册表核查:`HKCU\Environment` 两个 `CUTFORGE_*` 值已删;`HKCR\.cfproj`/`CutForge.Project` 已删
- [ ] 开始菜单/桌面快捷方式消失

## 5. 便携版路径(替代 §1,其余同)

- [ ] zip 解压到任意目录 → `bin\cutforge-cli serve --open` 直接可用
- [ ] 内嵌命中核查:`bin\cutforge-cli doctor <工程>` 的 ffmpeg 行标 `env`/`PATH`;zip 未带 ffmpeg 且机器没有时,
       doctor 给三选一修复指引(DEP_MISSING,不猜)
- [ ] 删除解压根目录 = 完全卸载(零注册表残留)

## 6. 结果归档

- [ ] 每项 ✅/❌ + 现象摘录 → `pure-checklist-run-<日期>.md`
- [ ] ❌ 项进 `docs/A6-PROGRESS.md` 遗留清单(修复后复跑对应节)
