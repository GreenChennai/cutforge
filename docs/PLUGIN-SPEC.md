# PLUGIN-SPEC · CutForge 插件规范(双形态统一契约)

> 册七 T7.2 / ADR-0024。技术选型与分级次序见 ADR-0024(Worker 先行 + 外部进程并行,
> WASM 远期带触发条件);本文是**双形态共用的契约面**:manifest 格式、权限模型、
> 调用协议、生命周期。JS Worker 宿主的壳侧实现是 G2 波(前端),本文对其同样生效。

## 一、双形态

| | Worker 形态 | 外部进程形态 |
|---|---|---|
| 代码跑在哪 | 壳内 Web Worker(独立全局、无 DOM 全权、postMessage 单通道) | 独立可执行/脚本进程(stdio JSON 协议,与 MCP 通道同风格) |
| 能力面 | 受限 API 子集(经宿主代理 `/api/v1`,token 不下发插件代码) | 能力最大:本地命令、工程外文件、ffmpeg 等重型依赖 |
| UI 贡献点 | 命令/右键菜单/面板/导入导出处理器(声明式清单 + 消息回调) | 无 UI 面(CI/批处理/CLI 场景) |
| 适用场景 | 编辑器内一切交互扩展 | headless 自动化、批处理、外部集成(唯一可选形态) |

两形态**共享同一套 manifest 权限模型与写通道纪律**:一切工程写操作必须走 Op 通道
(actor=plugin,OpLog 留痕可撤销)——Worker 经受限 API 面、外部进程经协议面,
两态汇入同一 `dispatch` 单表,不建第二套业务逻辑。

## 二、manifest 格式

契约唯一真相源:`schemas/plugin-manifest.schema.json`(Rust 侧经 cutforge-schema
编译期嵌入;服务端校验 = `plugin_validate` 工具 / `cutforge_mcp::validate_manifest`)。

```json
{
  "id": "demo-clip",
  "name": "示例插件",
  "version": "1.0.0",
  "form": "process",
  "entry": "plugin.py",
  "description": "一句话说明(首次启用确认时展示)",
  "permissions": {
    "read": true,
    "write": true,
    "network": false,
    "filesystem": ["01_原始素材"],
    "exec": false
  },
  "contributes": {
    "commands": [{ "id": "boost", "title": "整体提速 1.15x" }],
    "panels": [{ "id": "mix", "title": "混音面板" }]
  }
}
```

字段纪律:

- `id`:小写字母开头的 `[a-z0-9-]`,≤64 位;**安装目录名必须与 id 一致**;
  OpLog 归因 `actor=plugin(<id>)` 以此为准。
- `version`:三段数字 `x.y.z`;不做范围表达式(个人自用场景,升级=改数字)。
- `form`:`worker` | `process`。
- `entry`:安装目录内相对路径;`worker` 必须 `.js/.mjs`;`process` 必须带扩展名;
  拒绝绝对路径 / 盘符 / `..`(安装目录内寻址)。
- `permissions`:缺省一律 `false`/空 = 未声明即无权限;五类语义见下节。
- `contributes`:可选;Worker 形态的 UI 贡献点;`commands`/`panels` 的 `id` 不得重复。

## 三、权限模型(服务端裁决面)

**声明五类**:`read`(查询工具)/ `write`(工程写)/ `network`(网络)/
`filesystem`(工程内目录白名单)/ `exec`(编排与执行)。

**服务端裁决规则**(外部进程形态每次 `plugin-call` 都裁决;Worker 形态由宿主代理同规则把守):

| 工具分类 | 要求的声明 | 依据 |
|---|---|---|
| `query`(查询) | `permissions.read: true` | 读面显式授权 |
| `write`(写) | `permissions.write: true` | 写必须走 Op 通道,actor=plugin 留痕可撤销——插件无旁路 |
| `orchestrate`(编排) | `permissions.exec: true` | 编排会拉起脚本/子进程 |

- 越权拒绝:`GUARD_FAILED`,message 以 `FORBIDDEN:` 开头(5.4 表内码 + 语义前缀)。
- `network` / `filesystem` 是声明与审计面:Worker 形态由宿主代理执行(请求只许发往
  `/api/v1`);process 形态按生命周期约定在**首次启用确认**时展示给用户,manifest
  即承诺。服务端机械执行的是上表三条;目录白名单的强校验随 G2 壳侧 Worker 宿主落地。
- manifest 校验:`cutforge_mcp::plugin_validate` 工具(查询),或
  `plugin-call` 装载时自动校验;schema 面拒 = `SCHEMA_INVALID`。

## 四、调用协议(外部进程形态)

```bash
cutforge-cli plugin-call <manifest.json> <tool> --args-json '{"root": "…", …}'
```

1. 装载 manifest 并校验(不合法 → `SCHEMA_INVALID`,逐条 errors);
2. 权限裁决(越权 → `GUARD_FAILED`/`FORBIDDEN:…`);
3. 以 `actor=plugin(<id>)` 调用与 stdio/HTTP 同一个 `dispatch` 单表;
4. stdout 原样输出结果协议 envelope(`{ok, code, ns, message, data}`),退出码按 code 族归位
   (OK=0;NO_CONFIG/DEP_MISSING=3;其余失败=2)。

OpLog 归因实证:插件写入的 Op `actor.kind = "plugin"`、`actor.id = <manifest.id>`,
可经 `oplog_tail --actor plugin` 对账;撤销与普通编辑完全一致。

## 五、生命周期(目录约定)

- **安装**:把插件目录放进任一插件目录(`CUTFORGE_PLUGINS` 环境变量指向的目录,
  或壳侧设置面选择的插件目录),目录名 = `id`;目录内必须含 `manifest.json` 与
  `entry` 指向的入口文件。无注册表、无安装器——**目录即安装**。
- **启用/禁用**:配置开关(壳侧设置面;CLI 面不加载未启用插件)。首次启用必须经
  **确认对话框**(展示 name/version/description/权限五面)——Worker 与 process 同规。
- **卸载**:删除目录。无残留状态(manifest 即全部事实)。
- **升级**:改目录内容 + 递增 `version`(幂等装载,无迁移)。

## 六、写通道纪律(为什么插件造不成"影子工程")

插件与 AI、壳、CLI 共用唯一写入口 `Workspace::apply` 八步(前置检查 → 变更 →
schema 校验 → Op 入 OpLog → 持久化)。因此:

1. 插件的每次写都有 Op(before/after/causedBy/actor=plugin);
2. 撤销栈对插件写同样生效(非 auto Op);
3. 批量改动建议走 `preview_plan` 预演 + `apply_plan` 批准应用(册七 T7.5),
   插件可以只提案、由人批准——权限面与批准面正交。
