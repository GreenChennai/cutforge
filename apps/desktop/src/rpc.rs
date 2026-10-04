//! /rpc 与数据面 GET 客户端(壳内唯一网络收口;blocking,只在后台线程调用)。
//!
//! 契约(与 Web 壳 api.js 同口径):
//! - POST /rpc 体 = MCP `tools/call`:`{jsonrpc:"2.0", id, method:"tools/call",
//!   params:{name, arguments:{root, …}}}`(root=工程根,每调用注入);
//! - 响应 Envelope 在 `result.content[0].text`(字符串化 JSON);
//! - 事件长轮询 `GET /events?root=<enc>&since=<seq>`(≤1s 内有变化立即返回);
//! - 只读工具超时 10s,渲染类 300s;Envelope 形状 `{ok, code, message?, data?}`。

use std::time::Duration;

use serde_json::Value;

pub struct Rpc {
    agent: ureq::Agent,
    base: String,
    token: String,
    /// 工程根绝对路径(每次 tools/call 注入 arguments.root)
    root: String,
}

/// 渲染类工具(与 Web 壳 RENDER_CLASS 同清单;超时 300s)。
/// preview_zone_render = I1-M2 zone 预渲,同步渲染工具,同属渲染类。
fn is_render_class(tool: &str) -> bool {
    matches!(
        tool,
        "render" | "render_run" | "export_jianying" | "render_frame" | "preview_zone_render"
    )
}

impl Rpc {
    pub fn new(base: String, token: &str, root: String) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            .build();
        Rpc {
            agent,
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            root,
        }
    }

    fn auth(&self, req: ureq::Request) -> ureq::Request {
        if self.token.is_empty() {
            req
        } else {
            req.set("Authorization", &format!("Bearer {}", self.token))
        }
    }

    /// POST /rpc(tools/call)→ Ok(envelope.data) | Err(可展示消息)。
    pub fn call(&self, tool: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let url = format!("{}/rpc", self.base);
        let mut arguments = params;
        if let Some(obj) = arguments.as_object_mut() {
            obj.entry("root")
                .or_insert_with(|| Value::String(self.root.clone()));
        } else {
            arguments = serde_json::json!({ "root": self.root });
        }
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": arguments },
        });
        let resp = self
            .auth(self.agent.post(&url))
            .timeout(timeout)
            .set("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| format!("{tool} 请求失败:{e}"))?;
        let json: Value = resp
            .into_json()
            .map_err(|e| format!("{tool} 响应解析失败:{e}"))?;
        // MCP 结果面:envelope 字符串藏在 result.content[0].text
        let content = json
            .pointer("/result/content/0/text")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!(
                    "{tool}: MCP 响应缺 content(jsonrpc error:{})",
                    json.pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("无")
                )
            })?;
        let envelope: Value =
            serde_json::from_str(content).map_err(|_| format!("{tool}: envelope 非法 JSON"))?;
        parse_envelope(&envelope, tool)
    }

    /// 数据面 GET(白名单:/session /ui-fields /events /media /catalogs)。
    pub fn get(&self, path: &str, timeout: Duration) -> Result<Value, String> {
        let url = format!("{}{}", self.base, path);
        let resp = self
            .auth(self.agent.get(&url))
            .timeout(timeout)
            .call()
            .map_err(|e| format!("GET {path} 失败:{e}"))?;
        let json: Value = resp
            .into_json()
            .map_err(|e| format!("GET {path} 解析失败:{e}"))?;
        Ok(json)
    }

    /// 事件长轮询单次(0.9s 内核等待 + 传输余量;与 Web 壳 pollOnce 同口径)。
    pub fn poll_events(&self, since: u64) -> Result<Value, String> {
        let path = format!("/events?root={}&since={since}", percent_encode(&self.root));
        self.get(&path, Duration::from_secs(5))
    }

    /// 就绪探针:只打静态数据面 `/ui-fields`,HTTP 200 即服务可用——
    /// **不要求工程合法**(工程校验错误属于业务面,开窗后在状态栏展示,
    /// 不能让壳为坏工程白等健康超时)。
    pub fn probe(&self) -> Result<(), String> {
        self.get("/ui-fields", Duration::from_secs(2)).map(|_| ())
    }

    /// 内核返回的文件路径补全为绝对路径(media_thumbnail 给工程内相对路径,
    /// render_frame 给绝对路径;壳侧读文件前统一落绝对)。
    pub fn absolutize(&self, path: &str) -> std::path::PathBuf {
        let p = std::path::Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::path::Path::new(&self.root).join(p)
        }
    }
}

/// 最小百分号编码(路径安全集;不引 url 依赖)。
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b'\\' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Envelope → Ok(data) / Err(message)。
fn parse_envelope(env: &Value, tool: &str) -> Result<Value, String> {
    if env.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        Ok(env.get("data").cloned().unwrap_or(Value::Null))
    } else {
        let code = env.get("code").and_then(Value::as_str).unwrap_or("UNKNOWN");
        let msg = env
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("无错误详情");
        Err(format!("{tool} 失败[{code}]:{msg}"))
    }
}

pub fn render_timeout(tool: &str) -> Duration {
    if is_render_class(tool) {
        Duration::from_secs(300)
    } else {
        Duration::from_secs(10)
    }
}

// ---------------------------------------------------------------------------
// 单测(A-07 / TC-DESK-RPC-001):envelope 解析与超时分类,表驱动
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 表驱动:envelope 样例 → 期望(Ok(data) 或 Err(消息))。
    #[test]
    fn parse_envelope_table_driven() {
        let cases: Vec<(&str, Value, Result<Value, String>)> = vec![
            (
                "ok+data 对象",
                json!({"ok": true, "data": {"files": []}}),
                Ok(json!({"files": []})),
            ),
            ("ok 缺 data → Null", json!({"ok": true}), Ok(Value::Null)),
            (
                "ok+data 数组",
                json!({"ok": true, "data": [1, 2]}),
                Ok(json!([1, 2])),
            ),
            ("ok+data 标量", json!({"ok": true, "data": 7}), Ok(json!(7))),
            (
                "失败带 code+message",
                json!({"ok": false, "code": "E_LOCK", "message": "工程被锁"}),
                Err("t 失败[E_LOCK]:工程被锁".into()),
            ),
            (
                "失败缺 code → UNKNOWN",
                json!({"ok": false, "message": " boom"}),
                Err("t 失败[UNKNOWN]: boom".into()),
            ),
            (
                "失败缺 message → 无错误详情",
                json!({"ok": false, "code": "E_X"}),
                Err("t 失败[E_X]:无错误详情".into()),
            ),
            (
                "失败全缺 → UNKNOWN+无错误详情",
                json!({"ok": false}),
                Err("t 失败[UNKNOWN]:无错误详情".into()),
            ),
            (
                "ok:null 视为失败",
                json!({"ok": null, "code": "E_N"}),
                Err("t 失败[E_N]:无错误详情".into()),
            ),
            (
                "空对象视为失败",
                json!({}),
                Err("t 失败[UNKNOWN]:无错误详情".into()),
            ),
            (
                "message 非字符串 → 无错误详情",
                json!({"ok": false, "code": "E_T", "message": 3}),
                Err("t 失败[E_T]:无错误详情".into()),
            ),
            (
                "失败也带 data(忽略)",
                json!({"ok": false, "code": "E_D", "message": "m", "data": {"x": 1}}),
                Err("t 失败[E_D]:m".into()),
            ),
            (
                "ok 字符串 \"true\" 视为失败(严格 bool)",
                json!({"ok": "true", "code": "E_S"}),
                Err("t 失败[E_S]:无错误详情".into()),
            ),
            (
                "code 非字符串 → UNKNOWN",
                json!({"ok": false, "code": 5, "message": "m"}),
                Err("t 失败[UNKNOWN]:m".into()),
            ),
            (
                "data null 显式",
                json!({"ok": true, "data": null}),
                Ok(Value::Null),
            ),
        ];
        for (name, env, want) in &cases {
            let got = parse_envelope(env, "t");
            let matches = match (&want, &got) {
                (Ok(w), Ok(g)) => g == w,
                (Err(w), Err(g)) => g == w,
                _ => false,
            };
            assert!(
                matches,
                "case {name} 结果分支不符:want={want:?} got={got:?}"
            );
        }
        assert!(cases.len() >= 15, "表驱动用例数 ≥15,当前 {}", cases.len());
    }

    #[test]
    fn render_timeout_classification() {
        // 渲染类 300s
        for tool in [
            "render",
            "render_run",
            "export_jianying",
            "render_frame",
            "preview_zone_render",
        ] {
            assert_eq!(render_timeout(tool), Duration::from_secs(300), "{tool}");
        }
        // 普通工具 10s
        for tool in ["clip_add", "undo", "project_get", "media_browse"] {
            assert_eq!(render_timeout(tool), Duration::from_secs(10), "{tool}");
        }
    }

    #[test]
    fn percent_encode_safe_passthrough_and_escaping() {
        assert_eq!(percent_encode("/a/B_1.2~x\\"), "/a/B_1.2~x\\");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("中文"), "%E4%B8%AD%E6%96%87");
        assert_eq!(percent_encode("a&b=c?"), "a%26b%3Dc%3F");
    }
}
