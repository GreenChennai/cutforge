// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 免开工作区工具处理器(E6-3/B14:只读/创建/派生物缓存/目录级写面,
//! 不持排他锁、不做幂等预检)。A-01 自 dispatch.rs :113-154 逐字迁移;
//! 实现体各归其位(tools_nolock/media_tools/grade_tools/pro_ops/library_tools/
//! export_tools/media_library/pkg_tools),本文件只承载注册表接线。

use crate::handlers::{CommandHandler, HandlerCtx, Stage};
use serde_json::Value;

/// 免锁处理器接线宏:工具名 + 实现函数(参数形态统一 `(&Path, &Value)`)。
macro_rules! nolock_tool {
    ($struct_name:ident, $tool_name:literal, $impl:expr) => {
        pub(crate) struct $struct_name;

        impl CommandHandler for $struct_name {
            fn name(&self) -> &'static str {
                $tool_name
            }

            fn stage(&self, _args: &Value) -> Stage {
                Stage::NoLock
            }

            fn handle(&self, args: &Value, cx: &mut HandlerCtx) -> Value {
                let f: fn(&std::path::Path, &Value) -> Value = $impl;
                f(cx.ws_root, args)
            }
        }
    };
}

// 壳/探测面(T1.1 起)
nolock_tool!(
    ProjectNew,
    "project_new",
    crate::tools_nolock::project_new_tool
);
nolock_tool!(
    MediaProbe,
    "media_probe",
    crate::tools_nolock::media_probe_tool
);
nolock_tool!(
    MediaBrowse,
    "media_browse",
    crate::tools_nolock::media_browse_tool
);
// render_probe/stage_status 只读目录根,不吃 args(E6-3 起不为此申请排他锁)
pub(crate) struct RenderProbe;
impl CommandHandler for RenderProbe {
    fn name(&self) -> &'static str {
        "render_probe"
    }
    fn stage(&self, _args: &Value) -> Stage {
        Stage::NoLock
    }
    fn handle(&self, _args: &Value, cx: &mut HandlerCtx) -> Value {
        crate::tools_nolock::render_probe_tool(cx.ws_root)
    }
}
pub(crate) struct StageStatus;
impl CommandHandler for StageStatus {
    fn name(&self) -> &'static str {
        "stage_status"
    }
    fn stage(&self, _args: &Value) -> Stage {
        Stage::NoLock
    }
    fn handle(&self, _args: &Value, cx: &mut HandlerCtx) -> Value {
        crate::tools_nolock::stage_status_tool(cx.ws_root)
    }
}

// 册四 T4.1/T4.8 媒体池/音频工具(派生物缓存与纯计算,不改工程 IR 不持锁)
nolock_tool!(
    MediaPeaks,
    "media_peaks",
    crate::media_tools::media_peaks_tool
);
nolock_tool!(
    MediaThumbnail,
    "media_thumbnail",
    crate::media_tools::media_thumbnail_tool
);
// BUG-19:批量缩略图(一次请求多帧,磁盘缓存命中合并;壳侧 4N 次 IPC → 1 次)
nolock_tool!(
    MediaThumbs,
    "media_thumbs",
    crate::media_tools::media_thumbs_tool
);
nolock_tool!(
    MediaProxy,
    "media_proxy",
    crate::media_tools::media_proxy_tool
);
nolock_tool!(
    AudioBeats,
    "audio_beats",
    crate::media_tools::audio_beats_tool
);

// 册五 T5.2/T5.3/T5.6:调色 LUT/示波器/响度计/编码探测(同口径免锁)
nolock_tool!(LutImport, "lut_import", crate::grade_tools::lut_import_tool);
nolock_tool!(ScopeData, "scope_data", crate::grade_tools::scope_data_tool);
nolock_tool!(
    AudioLoudness,
    "audio_loudness",
    crate::grade_tools::audio_loudness_tool
);
nolock_tool!(
    EncodeProbe,
    "encode_probe",
    crate::grade_tools::encode_probe_tool
);

// 册五 T5.4/T5.5:多机位同步分析(纯计算)/ OTIO 导入(从零建工程,免锁)
nolock_tool!(
    MulticamSync,
    "multicam_sync",
    crate::pro_ops::multicam_sync_tool
);
nolock_tool!(OtioImport, "otio_import", crate::pro_ops::otio_import_tool);

// 册六 T6.1:布局迁移(工程目录)/ 工程库与崩溃恢复(root = 库根;免锁面,
// 目录级操作由 io 层自持锁)
nolock_tool!(
    MigrateLayout,
    "migrate_layout",
    crate::library_tools::migrate_layout_tool
);
nolock_tool!(
    LibraryManage,
    "library_manage",
    crate::library_tools::library_manage_tool
);
nolock_tool!(
    LibraryList,
    "library_list",
    crate::library_tools::library_list_tool
);
nolock_tool!(
    LibraryRecover,
    "library_recover",
    crate::library_tools::library_recover_tool
);

// 册六 T6.3:导出前检查(轻探测)/ 多画幅批量(编排入队;均免开工作区)
nolock_tool!(
    ExportPreflight,
    "export_preflight",
    crate::export_tools::export_preflight_tool
);
nolock_tool!(
    ExportAllVariants,
    "export_all_variants",
    crate::export_tools::export_all_variants_tool
);

// 册六 T6.2:素材库 manifest(库根)+ 素材拷贝导入(工程;免开工作区,
// 不产 Op 不改 IR,与 lut_import 同类写面)
nolock_tool!(
    MediaLibraryTool,
    "media_library",
    crate::media_library::media_library_tool
);
nolock_tool!(
    MediaImport,
    "media_import",
    crate::media_library::media_import_tool
);

// 册七 T7.6:.cfpkg 工程打包/解包(免开工作区;打包持锁在 io 层自持,
// 解包写全新目录拒绝覆盖——目录级写面,不产 Op 不改 IR)
nolock_tool!(
    ProjectPackage,
    "project_package",
    crate::pkg_tools::project_package_tool
);
nolock_tool!(
    ProjectUnpackage,
    "project_unpackage",
    crate::pkg_tools::project_unpackage_tool
);
