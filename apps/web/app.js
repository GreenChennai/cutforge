/* CutForge 兼容别名桩(册二 T2.7 / B-R1 回退期设施,ADR-0011)。
 *
 * 旧壳三文件已原样迁往 legacy/(经 /assets/legacy/index.html 访问)。
 * 根别名 /app.js、/style.css 的 HTTP 契约仍被 e2e_static 兼容红线锁定
 * (必须 200、Content-Type 同旧、与 apps/web 同名文件字节一致),因此
 * 这两个文件保留在原位;新壳入口是 /assets/js/main.js,本文件不被任何
 * 页面加载,仅承担根别名契约与迁移指路。 */
