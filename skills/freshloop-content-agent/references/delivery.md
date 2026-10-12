# 成品交付、报告与独立审查

本规则适用于早间、晚间及周报。成功交付不是稿件提交成功，也不是TTS处于queued/leased。

## 固定项目与角色

定时生产工作目录固定为 `/Users/jinliang/workspace/freshloop-content`，项目名“FreshLoop 内容生产”。不得创建项目、工作树、切回代码仓库或修改调度/服务。所有私有运行文件使用 `.task-work/agent-runs/日期/槽位`。
服务入口仍为 `python3 scripts/freshloop_agent_runtime.py`（预置链接，不复制凭据）。按原date/slot/state续跑，不能通过新ID重复发布。

主编选题、核实多模态、编排、维护租约、修稿并独占提交。使用Antigravity原生`invoke_subagent`调用两个已安装的自定义代理，workspace选`inherit`，不能创建项目：
- `freshloop-fact-auditor`：独立检查证据、数字、归因、事件覆盖与去重。
- `freshloop-listening-editor`：独立检查主持人衔接、听感、发音与正文清洁度。

给两位审查员明确稿件及源证据文件路径；它们没有发布权限，只返回审查结果。主编等待两者结束，修复blocking并复核。记录实际调用与结果，不能仅在报告声称“团队审查”。若运行环境不提供该工具，记录不可用并独立完成第二遍审查。

### 最终稿审查绑定

先完成机械校验和修稿，按下文准备原始输入与终稿对照，再让两名审查员读取完整来源和最终文件。以下旧形状只说明基础字段，必须补充下文的 input_file/input_sha256/comparisons；仅有摘要与 pass 已不能提交。将真实结果保存到 `review.json`：

```json
{"artifact_sha256":"最终artifact的指纹","mode":"native_subagents","reviews":[{"role":"facts","blocking":[],"checked_artifacts":["radio_program.json"],"summary":"实际核查结论"},{"role":"listening","blocking":[],"checked_artifacts":["radio_program.json"],"summary":"实际核查结论"}]}
```

指纹计算：`hashlib.sha256(json.dumps(artifact, sort_keys=True, ensure_ascii=False).encode()).hexdigest()`。
让审查员在读取文件时记录指纹，并核对两个结果属于同一版本。不要把旧结果绑定到新指纹。
提交增加 `--review-file review.json`；缺失记录、指纹不匹配或存在blocking会在提交前失败。
任何后续变更（包括修正校验错误、补证据和增删故事）都使审查过期，必须重审，不能仅重算指纹。
原生工具不可用时用 `mode=self_review` 并填写 `unavailable_reason`，保留事实和听感两遍实际复核结果。
审稿需特别检查同句复读、套话凑字数、故事被拼进正文两次、仅凭页面前1000字符或侧栏标题作结论。
读取正文必须定位真实文章，证据必须是原文片段，不能把自己写的中文稿回填为原文证据。

官方`/boost`面向复杂工程推理；`/teamwork-preview`需要前置范围确认且会创建独立工作区。不要在每次定时新闻任务里启动这两种流程，避免新项目、等待人工及额外调度漂移；工程故障调查可另行在获授权的维护任务使用。Pro主编与原生只读子代理是本生产流程的默认方案，不把Pro称为Boost。

## 发布后的交付闸门

1. 收集每次submit返回的真实voice_job_id，不从标题猜ID。先核对本周周报；缺失且已到周一08:00后，按原周槽位补做。已有周报也要检查Reading关联及音频。
2. 写`delivery-manifest.json`，例：

```json
{"date":"2026-10-09","edition":"早间","outputs":[{"title":"早间新闻联播","voice_job_id":"实际ID"},{"title":"Reading文章","voice_job_id":"实际ID"}],"warnings":[]}
```

若确实没有新增内容，outputs可为空，但必须写明no_new_content_reason并仍核对周报；通过后无新内容则按原规则安静结束。
outputs包括本次所有应有音频（Radio、Reading、补做周报），不能只列已成功的任务。warnings填写真实覆盖缺口、来源失败和修复情况，禁止编造百分百核查。
3. 调用：

```sh
python3 scripts/freshloop_agent_runtime.py finalize-run --manifest RUN/delivery-manifest.json --out RUN/delivery.json --report RUN/run-report.md --wait-seconds 5400
```

它等待真实voice job完成，核对产品记录的音频关联、正时长及公开MP3字节可访问；同时核对最近完整自然周周报及Reading关联。检查只读，不新建作业。
4. 命令未结束就仍在执行，必须继续等待；不要用“最多3分钟”等旧规则提前收尾。编辑阶段可控制在45分钟内，音频交付另给90分钟等待窗口，并保持工具任务可恢复。
5. **退出0且delivery.json中delivered=true**才能发送成功报告。退出2代表失败/未完成：读取证据，排查服务、队列、额度、失败任务或缺失周报，在原任务上恢复；必要时报告明确的交付失败和需处理事项，绝不声称完成后再附“不能保证可听”。期限不是成功条件。
6. 用户报告直接使用生成的run-report.md，不再自由发挥长篇技术报告。固定内容为：日期/版次、明确交付结论、内容/收听链接/时长表、周报核对、异常与未完成项。内部job ID、leased等状态留在delivery.json，不给听众解释数据库状态。

该闸门验证的是发布关联和音频网络可访问，不能代替全文ASR、真实客户端播放和事实审查，不得把它描述为全文语音质量保证。

### 原始输入与最终稿对照（必需）

机械校验通过不等于语义正确。两位审稿人必须拿完整来源正文与最终稿逐项比较；不能只阅读编辑自写摘要、标题、评分或另一位审稿人的结论。主编先运行：

```sh
python3 scripts/freshloop_agent_runtime.py review-inputs --artifact RUN/radio_program.json --sources RUN/sources_content/实际来源ID.json --out RUN/review-inputs.json
```

`RUN` 是本次真实私有运行目录。命令输出 input_file/input_sha256，输入文件列出每个终稿单元、对应 source_id、URL、原始快照指纹及完整正文 snapshot 路径。审稿人必须实际读取所有 snapshot 的 body 和终稿单元；不要按前500/1000字符截断。原文缺失、获取失败或不能确定正文时结论是 unverifiable，明确“无法完成来源核验”，不能推断正文。

每个审稿结果保留原有 role/blocking/checked_artifacts/summary，并增加 comparisons。每个新闻故事及完整板块的 script 与 audio_script 都需对照（包括衔接句，不能把新增事实藏在故事元数据之外）；Reading 要对照 reader_markdown、compressed_markdown、audio_script，Weekly 对照正文与可听稿。示例形状（值必须来自实际审查，不得照抄）：

```json
{"artifact_sha256":"最终稿指纹","input_file":"review-inputs.json","input_sha256":"准备命令输出的输入指纹","mode":"self_review","unavailable_reason":"实际能力限制","reviews":[{"role":"facts","blocking":[],"checked_artifacts":["radio_program.json"],"summary":"具体结论","comparisons":[{"unit":"/sections/0/stories/0/script","source_ids":["实际ID"],"checked_dimensions":["actor","event","time","numbers","limits","core_meaning","important_omissions","unsupported_additions","compression"],"conclusion":"pass","differences":[],"assessment":"主体、事件、数字口径、限制条件及重要意义如何被保留；可省略什么、为什么","evidence":[{"source_id":"实际ID","source_excerpt":"完整正文中的实际原句","draft_excerpt":"终稿中的实际原句","observation":"两者的具体对应关系"}]}]}]}
```

上例仅展示一行；实际记录必须含 facts/listening 两个角色及输入文件中的全部单元。conclusion 只能是 pass、revise、reject、unverifiable。有实质差异时 differences 必须列出 kind/reason，例如丢掉公司主体、数字比较口径变化、遗漏限制条件、添加来源没有的商业进展，或缩略后只剩抽象标题。不得一面有差异一面写 pass。引用与来源/终稿不符、漏审 audio_script、来源或稿件变化都会在提交前阻断；程序只验证输入绑定、结构、真实引用和处置，不把 LLM 的 pass 当成事实正确性的证明。

审稿完成后先运行 `review-check --state RUN/radio-state.json --artifact RUN/radio_program.json --review-file RUN/review.json`（使用真实原状态；生产目录强制带 --state，离线非发布样稿可省略），只检查来源绑定、引用和审稿记录；它不会发布、调用模型或生成音频。通过后再按原 submit 流程执行。程序输出 semantic_truth_verified_by_program=false，不能把自评结论当成语义自动测试通过。

重点新闻必须讲清主体、事件和来源支持的关键细节及边界；简讯可以短，但仍完整保留核心事实。不要用统一字数、压缩率或逐字复写要求代替语义审核。没有某类信息时说明不适用及理由，不编造背景。

### 异常、修订与保留

helper 的来源、校验、审稿及提交异常会向 stderr 输出结构化事件，并写本次 RUN/editorial-issues.json 和现有 run-report.md：稿件/来源标识、指纹、发生步骤、具体原因和处置。不可删除这个记录来恢复“成功”，不能捕获异常后返回空稿继续提交。失败稿与通过稿按指纹保留在 .editorial/artifacts，源正文快照同内容只保存一次，审核记录在 .editorial/reviews；没有另一套生产系统或重复大报告。

一个已租赁逻辑任务最多尝试三个不同的失败稿（初稿加最多两次修订）。达到上限后 helper 标记 editorial_status=failed 并禁止继续提交；主编在确认没有未知提交结果且仍持有效租约时，用原来的 fail --state ... --error ... 一次结束该任务并报告具体原因。不得新建ID绕过上限，不无限重试。重复同一失败稿不增加不同稿件计数，但必须处理或停止。

修订后重新准备输入并进行实际审稿，旧异常保留为 corrected_and_reviewed。重新取得来源只表示可核验，仍须语义复审。尚有未解决的文稿/来源异常时 finalize-run 立即退出2，把异常带进 delivery.json 与现有报告，即使其他音频已完成，也不能当作全次交付成功。

## 已发布错误稿的维护修复

不要用新slot重发。获授权的维护任务可通过原job的 `repair-radio` 接口提交修正稿和 `expected_voice_job_id`，保留卡片和原始证据，生成替换音频。普通定时任务发现已发布错误时保存证据并报告，不能自动覆盖未知版本。所有修复仍须最终稿复核、音频验收及上线关联验证。
