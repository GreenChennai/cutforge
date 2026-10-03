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
fn is_render_class(tool: &str) -> bool {
    matches!(
        tool,
        "render" | "render_run" | "export_jianying" | "render_frame"
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
