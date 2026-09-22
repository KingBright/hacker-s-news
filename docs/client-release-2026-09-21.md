# Web / Android 发布 — 2026-09-21

用户授权发布外驱模式配套客户端更新。使用唯一正式入口 `./scripts/deploy.sh --android`，一起构建并发布 Web、Android APK 与版本清单。NAS Nexus 和本地 Cortex 保持此前已验收的部署。

- Android：1.3.32+41，前一线上 build 为 40；保留本地尚未发布的新版本，没有重复增加版本号。
- 公网入口：https://news.hackerlife.fun:8443
- APK：https://news.hackerlife.fun:8443/android-app.apk
- APK SHA-256：`02cb9148f15efe4416debc6be2ae4417b7a718582db3923b68a4a4189e3810a8`
- 正式入口成功退出；公网 8443 下载回读通过版本、包名、标签、签名延续和 SHA-256 校验。
- 线上 Web 首页与本次 `frontend/out/index.html` 字节完全一致。
- Web lint、Web production build、Flutter analyze、品牌校验、33 项 Android 发布契约测试通过。
- Feed API smoke 通过（只读及未认证访问边界；未执行合成内容写入）。

本次没有连接 Android 真机做安装与后台播放测试；APK 发布校验不能替代真机运行验收。已安装用户可通过应用更新机制获取 build 41。

发布日志与浏览器验收证据保存在 `.task-work/client-release/`。保留工作区既有改动，未执行提交、重置或清理。

浏览器上线回归通过：Reading 周报原图加载（1456px），点击新版“播放周汇总”按钮后音频播放进度前进，时长 183.648 秒；桌面与手机截图已保存。390px 宽度 Reading、Radio、Loop、Focus 均无横向溢出，四个页面正常加载。旧验收脚本的英文按钮选择器已按新版中文标题调整，不涉及产品修改。
