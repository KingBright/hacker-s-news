# 外驱模式生产验收 — 2026-09-21

已部署 Nexus v2 与本地 Cortex，并切换到 `external_agent`。Cortex 负责抓取、缓存和 TTS；Antigravity 负责编辑生产。Radio / Reading / Loop / Focus 保留。本次没有部署前端或 Android，现有客户端保持原版本。

## 原生定时任务

时区 Asia/Shanghai；Antigravity 原生调度存在时间抖动，以下为计划时间。三个任务最终 UI 回读均为 enabled=true；临时验收时间均已撤销，无重复 Codex/cron 调度。

| 任务 | 计划时间 | 验证 |
| --- | --- | --- |
| freshloop-loop | 每日 07:30 | 原生触发、真实发布、14 条音频完成 |
| freshloop-evening-loop | 每日 18:00 | 原生触发、真实发布、10 条音频完成 |
| freshloop-weekly | 每周一 08:00 | 原生触发、重复周检测、修复与音频验收完成 |

实际提示词保存于 `antigravity-schedules.json`。任务复用稳定逻辑 ID、维护租约、逐篇审核提交；日常任务也检查遗漏周报。语音等待限定三分钟，未完成 ID 留存供下次核对。每周验收包含人工事实复核及显式修复提交，并非声称首次无人干预生成已完美通过。

## 实际产出验收

最终实时查询：25 个语音任务全部 completed，产品记录音频 URL 全部一致，总计约 42.1 分钟。早间 11 条 Radio + 3 条 Reading（26.4 分钟），晚间 10 条 Radio（12.6 分钟），周报 1 条（3.1 分钟）。

周报覆盖北京时间 2026-09-14 至 09-20。原记录存在英文模板残留且无音频；经 Antigravity 重写和第二轮人工事实核查，移除无来源的成本和因果推断，形成 1052 字文稿。显式修复保留原周报及 Feed ID，旧语音任务被隔离，替换语音首次成功。最终确认同周仅一条周报，关联 Feed 音频一致。

周报实测音频 183.648 秒；首尾各 25 秒 ASR 抽查与文稿流程吻合（专名识别有通常误差）。浏览器确认原图加载、真实点击后音频播放进度前进；390px 手机宽度无横向溢出。三篇 Reading 原文模式的脚本/CSS/导航杂质已清理，保留正文、绝对图片地址、视频来源链接及音频元数据；最终 API 回读未发现已知杂质。并未声称对所有 25 篇全文做了独立事实审计。

试听：[已验收周报](https://news.hackerlife.fun:8443/audio/curated_weekly_22e93f5a-6547-49df-bbfd-7861188451f7.mp3)

## 稳定性与性能

- 本地状态：external_agent_enabled=true，voice_worker_enabled=true；本地内容、Reading、周报与 Loop LLM 自动生成关闭。
- TTS 保留两路并发、最多两个模型进程；已有 1/2/3 路实测显示第三路更慢，详见 `tts-throughput-2026-09-21.md`。
- 语音失败指数退避 30–3600 秒。关闭未经编辑审查的历史缺音频自动回填；新稿原子创建语音任务及常规重试保持运行，历史记录未删除。
- 发现公网回环偶发超时。Cortex 与运行助手对 Nexus 使用进程内 LAN 地址解析，保留原域名 TLS/SNI 和证书校验，未修改全局 DNS。连续 20 次认证 HTTPS 请求通过；该路径要求电脑可访问家庭内网，并不意味着公网回环问题已全局消失。
- Antigravity 的 Prevent Sleep 和 Keep In Menu Bar 已启用。定时执行仍依赖应用运行、电脑可用、内网和模型账号服务可用；目前验收是实际触发及端到端结果，不是长期零故障保证。

## 检查与证据

- Cortex：83 项测试及辅助检查通过；Nexus：38 项二进制测试和 1 项库测试通过。
- 编辑助手 7 项、运行助手 2 项测试通过。
- Feed API smoke（LAN 路由）通过；真实发布验证了写入链路。
- 最新 Cortex 安装器成功，6 段 ASR 平均 99.37%、最低 98.01%，PASS；对构建二进制应用同名签名后，与安装文件逐字节一致，服务正常运行。
- Nexus 部署遇 SSH 收尾超时后已恢复服务，远端二进制 SHA 与构建一致，健康检查通过。
- 私有证据：`.task-work/external-rollout/` 下的 `final-audio-audit.json`、`lan-reliability.json`、`weekly-asr-acceptance.json`、`weekly-browser.log`、桌面/手机截图及安装日志。备份位置和恢复过程记录于该目录 checkpoint。
