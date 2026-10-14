# 主持人、语气与 TTS 降级

在编排 Radio/Weekly 主持人、第一次使用新语音配置、或音频任务异常时读取。
Cortex 决定音色和生成路线；Agent 写稿、核实和验收，不能靠重复发布触发换模型。

## 固定人物与分工

以 `tts-status --out RUN/tts-status.json` 返回的实时 `policy.hosts` 为准。
当前五位主持人保持名字与板块不变；开场、收尾归第一位 Vivian。

| 主持人 | 板块 | Gemini 预置音色 | 表达性格 |
| --- | --- | --- | --- |
| Vivian | AI前沿、技术产业、生命健康；开场、收尾 | Kore | 沉稳温暖、条理清晰，讲清因果，健康内容审慎 |
| 灿灿 | 硬件数码、游戏电竞 | Aoede | 轻快敏锐、有工程师的好奇心；不喊叫、不做营销 |
| 小萌 | 影音文娱 | Leda | 亲切细腻、有文化观察力；不追星式夸张 |
| 小凡 | 商业财经、国际时政 | Charon | 冷静理性、重事实来源和数字；保留不确定性 |
| 小何 | 科学探索、生活杂谈、其他 | Sadaltager | 博学好奇、耐心解释；不把假说说成定论 |

名字是节目身份，voice 是供应商音色标识，personality 是表达方向。人物性格
不是虚构履历、专业资质、私人经历或立场，不得添加到新闻事实中。争议、灾难、
健康和财经内容不因角色活泼而变成戏谑或耸动。稿件保持主次和覆盖完整。

当前 Radio artifact 仍只提交 category/script/audio_script；不要增加未经后端支持的
speaker、voice 文件路径或 style 字段。服务读取 hosts 的 gemini_voice/personality，
把性格传入独立的 speech_metadata，不朗读这些指令。保留一次开场、自然板块交接
和一次收尾。原生 Gemini 对话最多两人；三人以上是逐轮合成，并非原生群聊。
`check_gemini` 是独立试听工具，不经过生产任务路由；它报429后应停止试听，不会自动启动本地克隆。不要把试验用 Dialogue JSON 提交到当前 radio_program 字段。Weekly 仍沿用已有
weekly_digest 合约；未上线的新对谈板块不能由定时 Agent 自行创造。

## 自动降级的真实边界

仅当实时 configured_engine=gemini 时启用云端路线。当前部署顺序是 Flash-Lite →
Flash → 本地 VoxCPM Metal，实时 policy 配置优先。相同 Gemini 预置音色和性格保留到另一云端模型；降到本地
则使用原有五位主持人的参考 WAV，人物身份/板块不变，实际音色可能改变。

| 故障 | 服务动作 | Agent 动作 |
| --- | --- | --- |
| HTTP 429：限流或额度不足 | 优先遵守Retry-After/RetryInfo；明确日额度且无重试时间时等太平洋午夜；分钟或未知限额无提示时短暂冷却60秒，当前任务考虑本地；不假定换模型能绕过额度 | 记录 Quota/cooldown_until；保持当前任务，不循环调用 Google，不换 key/项目绕限制 |
| HTTP 401/403、缺少 API key | Google共同暂停24小时，考虑本地，并记录 Auth | 报告凭据/权限需修复；不反复试密钥，不读取或打印秘密 |
| HTTP 408/5xx、连接/请求超时 | 当前模型冷却至少5分钟，按顺序尝试下一层，每层最多一次 | 超时可能已计费；不要另行启动相同语音请求 |
| 无效/不完整音频响应 | 当前模型冷却，考虑下一层 | 这是格式/状态校验失败，不等同于已做全文ASR |
| HTTP 400、稿件/配置/路径错误、未知本地错误 | 停止本次路线，不用换模型掩盖错误 | 保留文稿/voice job，报告具体边界，先修输入/配置 |
| 本地也失败或所有候选冷却 | 本次 voice job 失败，由 Nexus 既有有限重试处理 | 报告 audio_failed/等待重试，不建立新文章或新slot |

Retry-After 数字秒数会被参考并封顶24小时。无法可靠区分429是分钟限流还是日额度，
因此采用保守的共享冷却，不宣称自动判断了账单配额。冷却持久化，重启不是解限办法。
已在途的另一条任务可能仍收到错误；不会取消或篡改另一条任务的租约。

降级以整条 voice job 为单位：同一期节目不会混入 Flash/Lite/本地不同路线的片段。
同路线已完成板块缓存可复用；跨路线使用独立缓存。任务一旦选到降级路线，后续重试
仍固定在该层或更低层，避免中途重新升回主模型。新任务在冷却到期后才重新尝试上层。
云端/本地同时不可用时不生成空音频，不把排队描述为完成。本地最大两进程，不能为
赶进度再开一套TTS进程；降级时GPU重新繁忙是预期现象，应在报告说明。

## 操作与验收

1. 用 helper `tts-status --out RUN/tts-status.json` 读取实时策略及最近事件。
   supported=false 表示当前服务尚未提供新路由能力，不得声称自动降级已启用。
   policy 是配置；routing.last_event 是最近一条任务事件，先核对 job_id，不能代表全部任务。
2. 用原 voice_job_id 查询 Nexus `/api/internal/agent/voice-jobs/{id}`。分别报告
   manuscript_published、audio_queued、audio_completed、audio_failed。
   routing 的 synthesized 仅表示合成返回，仍需 Nexus complete 和可播放URL确认上传完成。
3. 在运行记录保留 job_id、实际观测到的 provider/degraded/reason/cooldown_until、
   完成状态与验收结果；没有该任务的 provider 证据就写 unknown，不从配置推断实际结果。
4. 音频成功仍需抽听/转写。新人物、新模型、长节目及降级节目优先核查全文，至少覆盖
   开头、中间、结尾、交接、名字和数字。整稿一次性提交可配置，但默认约400字按句拆分。
   2026-10-08 的1232字整段试验虽然HTTP成功且结尾存在，中段仍额外重复，不能只看结尾。
5. ASR应比对完整参考与完整识别序列，查新增/重复/遗漏。不要用“最佳匹配子串”分数证明
   完整性。ASR不是语气/音色/专名正确性的充分证据，无听取能力就明确未听验。
6. 当前路由只自动处理通信与音频格式失败，**不自动识别合法WAV里的语义重复**。如发现
   文稿重复、漏读、错人、严重错音，记录 audio_requires_review，停止把它报告为验收通过。
   不重新submit已完成文稿、不伪造失败租约、不删除线上音频或直接改数据库；请求维护流程
   针对原voice job修复。没有受支持的重做接口时明确需要维护，不能声称已自动修好。
7. 定时内容Agent不部署服务、不修改生产engine/主持人映射、不清空冷却状态、不创建额外
   调度器。用户明确提出维护操作时，按维护任务处理并保留原任务身份与可审计记录。

## 模型变化与 AI Studio 核查

遇到持续异常或准备迁移模型时，核对官方 speech-generation 文档、模型目录和
可访问的 Google AI Studio 模型选择/配额页面，记录检查日期与完整 model ID。
用户授权的已登录浏览器可用于只读核查；不要显示密钥、创建新凭据、改账单或项目。
Studio 所选项目与服务 API key 所属项目未证实相同前，不能把页面额度视为服务额度。
区分503/504/超时与429；临时服务故障不是模型退役证据。不能只凭模型名称或公告
自动替换生产配置。先核实 API/schema、音色与双人支持、长度限制，再做有限短稿、
双人及完整性验收；在429冷却期不额外发试听请求。浏览器不可用时如实记录限制，
沿用已验证配置和降级机制，不声称已经检查了页面。定时内容任务无权自行部署迁移。

## 请求预算、缓存与排程（2026-10-09）

`tts-status` 的 policy.request_budget / request_usage 是本机共享的编辑预算与计数，
不是 Google 项目全量用量。默认本地上限60次/北京时间自然日；失败/超时也计一次。
不统计本功能上线前、AI Studio 或其他应用调用，不能用于反推供应商剩余额度。
completed/audio_seconds 是成功合成请求及其秒数，不是已发布节目统计；attempts-completed
可能包含失败或在途请求。cache_hits 不花新请求，缓存命中不计新生成秒数。

Radio 与 curated_weekly 周报优先：上午请求保留25次晚间余量；下午可使用全部剩余预算。
Reading 等其他音频保留40次给早晚/周报。预留是全局剩余门槛，不是每份任务的配额保证。
周报仍按原定周一与日常补漏执行，不得因为云预算不足跳过稿件/音频；云端不足整条任务
降到本地。未来额外对谈需要另核预算，不能擅自扩大每日上限。

Gemini错误不参与本地片段的三次重试；统一交给整条任务路由决定降级。
请求跨进程串行化，开始时间至少间隔20秒；这只是保守节流，不保证符合尚未确认的RPM。
请求级缓存仅用于同一voice job恢复，键含job ID/模型/文本/音色/语气。Nexus确认任务完成后清理；失败片段超过24小时失效并在下次缓存访问时清理，不跨新闻任务复用。
有预算也先查缓存，不合并不同主持人的文本，不增加请求字数，不删掉简讯省额度。
LocalBudget 与 Google Quota 不同：前者不设置Google共享冷却，不妨碍高优先级任务使用预留。
定时Agent先读状态、估算ceil(字符数/400)的下限再留切分/失败余量；按原slot提交一次，
不得新建任务绕过预算。额度未知就报告未知，不能把本地60次写成Google额度。
缓存仅验证音频格式，不能证明全文无重复；发现语义缺陷仍需人工修订/针对缓存修复，
不能靠相同提交期待重新随机生成。

429不是统一的日限额错误。routing.last_event.quota_scope为daily/minute/unknown；unknown必须报告未知，不能从两次成功或长Retry-After推断日额度。每天40分钟云端生产需要核对实际项目层级与RPD/RPM/TPM，缓存和本地60次预算都不能提高供应商额度。若日额度不足，应调整付费容量或选择已获授权且有足够容量的云端供应商；本地降级只是保供，不是减轻本机负担目标的完成证明。定时Agent不自行开通付费、切换项目/密钥规避限额。

## Google Colab L4 双模型云端 GPU 混排与 A/B 轮换路线（2026-10-14）

当语音任务从本机 Mac Metal（`voxcpm_metal`）卸载至云端 L4 GPU（零本机 GPU 占用）时，采用 **VoxCPM2** 与 **IndexTTS 2.5** 双模型混排架构：

1. **职责边界与路由**：
   - 定时内容 Agent（`freshloop-content-agent`）只负责资讯核对、文稿生产、双盲独立审稿、`submit` 与 `finalize-run`，不绕过 Nexus 直接替换线上语音任务状态。
   - Cortex TTS 路由层与 `colab-operator` 编排器（`/Users/jinliang/workspace/hacker-s-news/skills/colab-operator/scripts/colab_tts_client.py`）共用相同的五位主持人参考音频（`audio/{vivian,cancan,meng,xiaofan,xiaohe}.wav`），并按以下 A/B 规则自动分流并在 `tts-routing.json` 中记录具体 provider（`colab_voxcpm2` 或 `colab_indextts25`）：
     - **早间 Radio (`morning-program`, 00:00–13:59 CST)**：使用 **`colab_voxcpm2`**（`openbmb/VoxCPM2` Ultimate Cloning 模式，英文技术术语发音精准）。
     - **晚间 Radio (`evening-program`, 14:00–23:59 CST)**：使用 **`colab_indextts25`**（`IndexTeam/IndexTTS-2.5`，情感韵律丰富，自动执行口播年份与英文缩略词分词保护）。
     - **周报 (`curated_weekly` / `weekly_digest`)**：使用 **`colab_indextts25`**。
     - **精读文章 (`reading_article`)**：按 `job_id` SHA256 奇偶性 50/50 确定性分流至 **`colab_voxcpm2`** 与 **`colab_indextts25`**。
2. **质量增强与生命周期防线**：
   - 云端统一执行器（`examples/cloud_tts_runner.py`）对两个模型均启用句子级安全切分（`<= 150~160` 字）、静音/畸变坏块重试、15ms/25ms 余弦淡入淡出防爆音与广播级响度归一化（Target RMS `0.085`）。
   - 多任务连续入队时通过本地文件锁与 `.waiting` 队列标记自动复用热启动的 Colab L4 会话；当队列排空时，在 `finally` 块立即执行 `colab stop -s <session>` 并核实 `Active assignments: 0`，杜绝算力空耗。


