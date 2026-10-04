//! 桌面壳视觉基础层(§9.4/§9.5/§9.7;工单 V2-W3-SHELLB):
//! - [`theme`]:三层设计 token(原始/语义/组件)+ sable ColorTokens 桥——
//!   **裸值唯一定义点**(门禁 TC-GATE-003 豁免本目录);
//! - [`icon`]:图标系统(Icon 枚举 + 内嵌 SVG 资产源)——**字形禁地**
//!   (门禁 TC-DESK-ICON-002);
//! - [`fx`]:动效时长梯度 + easing 三族 + reduced-motion 总控——
//!   **动效毫秒唯一定义点**(门禁 TC-GATE-004)。

pub mod fx;
pub mod icon;
pub mod theme;
