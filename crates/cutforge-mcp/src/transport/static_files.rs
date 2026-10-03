// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 静态托管目录化(T1.6/AC-1.6):`/assets/*` 直接映射 Web 资源目录(默认 apps/web),
//! Web 目录里新增文件零 Rust 改动、零重启即可访问;根路径四别名(/ /index.html
//! /app.js /style.css)行为保持不变(兼容红线:CI web-e2e 全绿是底线,apps/web 一行不动)。
//! canonicalize 穿越防护 + MIME 表 + ETag/304。gzip/br 压缩显式不做(本地回环传输,
//! 收益趋零;若要做须先补 ADR,见 ADR-0009 被否决的替代小节口径)。

use crate::transport::http::{HttpResp, mime_of, pct_decode, resp_plain};
use std::path::{Path, PathBuf};

/// 静态面路径判定:旧四别名 + `/assets/` 前缀白名单(其余路径不进静态面;
/// 数据面 /session /rpc /events /media 仍必须持 token,鉴权口径零变化)。
pub(crate) fn is_static_path(path: &str) -> bool {
    matches!(
        path,
        "/" | "/index.html" | "/app.js" | "/style.css" | "/assets"
    ) || path.starts_with("/assets/")
}

/// 静态响应:命中 → 200(带 ETag);If-None-Match 命中 → 304(空体);
/// 未命中/穿越 → 404。
pub(crate) fn static_resp(web: &Path, path: &str, if_none_match: Option<&str>) -> HttpResp {
    let file = match path {
        // 兼容别名:既有根路径行为不变(同文件经 /assets/<同名> 亦可访问)
        "/" | "/index.html" => Some(web.join("index.html")),
        "/app.js" => Some(web.join("app.js")),
        "/style.css" => Some(web.join("style.css")),
        p => p
            .strip_prefix("/assets/")
            .and_then(|rel| assets_resolve(web, rel)),
    };
    let Some(file) = file else {
        return resp_plain("404 Not Found", "not found");
    };
    let Ok(data) = std::fs::read(&file) else {
        return resp_plain("404 Not Found", "not found");
    };
    let etag = etag_of(&file, data.len());
    if let Some(inm) = if_none_match
        && inm.split(',').map(str::trim).any(|t| t == etag || t == "*")
    {
        // 304 不回体;ETag 必须随 304 回带(客户端下一轮条件请求要用)
        return HttpResp {
            status: "304 Not Modified",
            ctype: "text/plain".into(),
            extra: format!("ETag: {etag}\r\n"),
            body: Vec::new(),
        };
    }
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    HttpResp {
        status: "200 OK",
        ctype: mime_of_static(ext).to_string(),
        extra: format!("ETag: {etag}\r\n"),
        body: data,
    }
}

/// ETag 规则:强校验器 `"<len-hex>.<mtime_ms-hex>"`——文件长度或修改时间任一变化
/// 即视为新资源;命中即 304 不传体。静态件无需内容哈希(单机本地服务,
/// len+mtime 已足够;引入哈希徒增每次请求的读盘成本)。
fn etag_of(file: &Path, len: usize) -> String {
    let mtime = file
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("\"{:x}.{:x}\"", len, mtime)
}

/// `/assets/<rel>` 的安全解析,纵深防御三道闸:
/// ①百分号解码后逐段拒绝 `..`、盘符(`:`)与 NUL(穿越/绝对路径在字面层就死);
/// ②只按段重建路径,不吃原始串(%2e%2e 双编码、反斜杠混写一并拦下);
/// ③canonicalize 后必须仍位于 Web 目录之内(最终裁决,符号链接逃逸也出不去)。
/// 任一道闸不过 → None → 404。
fn assets_resolve(web: &Path, raw_rel: &str) -> Option<PathBuf> {
    let rel = pct_decode(raw_rel);
    if rel.is_empty() {
        return None;
    }
    let mut built = web.to_path_buf();
    for seg in rel.split(['/', '\\']) {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." || seg.contains(':') || seg.contains('\0') {
            return None;
        }
        built.push(seg);
    }
    let canon_web = web.canonicalize().ok()?;
    let canon_file = built.canonicalize().ok()?;
    if !canon_file.starts_with(&canon_web) || !canon_file.is_file() {
        return None;
    }
    Some(canon_file)
}

/// Web 静态件 MIME(文本类带 charset;媒体类复用 /media 的 `mime_of`,不建并行表)。
fn mime_of_static(ext: &str) -> &'static str {
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "txt" | "md" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ico" => "image/x-icon",
        other => mime_of(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cf-static-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(d.join("js")).unwrap();
        std::fs::write(d.join("index.html"), b"<html>ok</html>").unwrap();
        std::fs::write(d.join("app.js"), b"console.log(1);\n").unwrap();
        std::fs::write(d.join("js").join("x.js"), b"window.X=42;\n").unwrap();
        d
    }

    #[test]
    fn assets_maps_directory_and_rejects_traversal() {
        let web = scratch("map");
        // 目录化:新文件直接可达;子目录亦然
        assert!(static_resp(&web, "/assets/js/x.js", None).status == "200 OK");
        assert!(static_resp(&web, "/assets/index.html", None).status == "200 OK");
        // 穿越面:字面 / 编码 / 反斜杠 / 盘符 / 双编码 / 空段,100% 拒绝为 404
        for bad in [
            "/assets/../secrets.txt",
            "/assets/../../etc/passwd",
            "/assets/%2e%2e/secrets.txt",
            "/assets/..%2Fsecrets.txt",
            "/assets/..\\..\\secrets.txt",
            "/assets/%252e%252e/secrets.txt",
            "/assets/C:%5Cwindows/win.ini",
            "/assets/....//....//secrets.txt",
        ] {
            assert_eq!(
                static_resp(&web, bad, None).status,
                "404 Not Found",
                "{bad} 必须拒绝"
            );
        }
        // 404 与空 rel
        assert_eq!(
            static_resp(&web, "/assets/missing.js", None).status,
            "404 Not Found"
        );
        assert_eq!(static_resp(&web, "/assets/", None).status, "404 Not Found");
        assert_eq!(static_resp(&web, "/assets", None).status, "404 Not Found");
        std::fs::remove_dir_all(&web).ok();
    }

    #[test]
    fn etag_304_and_mime_and_compat_aliases() {
        let web = scratch("etag");
        // 兼容别名行为不变:内容一字节不差
        for (p, f) in [("/", "index.html"), ("/app.js", "app.js")] {
            let r = static_resp(&web, p, None);
            assert_eq!(r.status, "200 OK");
            assert_eq!(
                r.body,
                std::fs::read(web.join(f)).unwrap(),
                "{p} 内容必须与源文件一致"
            );
            assert!(r.extra.contains("ETag: "), "{p} 应带 ETag");
        }
        // ETag 命中 → 304 空体;未命中 → 200
        let etag = static_resp(&web, "/assets/js/x.js", None).extra;
        let r304 = static_resp(
            &web,
            "/assets/js/x.js",
            Some(etag.trim().strip_prefix("ETag: ").unwrap()),
        );
        assert_eq!(r304.status, "304 Not Modified");
        assert!(r304.body.is_empty());
        assert_eq!(
            static_resp(&web, "/assets/js/x.js", Some("\"dead.beef\"")).status,
            "200 OK"
        );
        // MIME:js 带 charset,未知扩展回退 octet-stream(复用 /media 表)
        let js = static_resp(&web, "/assets/js/x.js", None);
        assert_eq!(js.ctype, "text/javascript; charset=utf-8");
        std::fs::write(web.join("js").join("blob.bin"), b"x").unwrap();
        assert_eq!(
            static_resp(&web, "/assets/js/blob.bin", None).ctype,
            "application/octet-stream"
        );
        std::fs::remove_dir_all(&web).ok();
    }
}
