# FreshLoop 管线批量诊断报告

- 分析时间: 2026-02-09 10:34
- Trace 数量: 51

## 总体统计

| 指标 | 值 |
|------|-----|
| 聚类成功率 | 41/51 (80%) |
| 编排成功率 | 44/51 (86%) |
| 平均素材覆盖率 | 83% |
| 平均复制粘贴率 | 37% |
| 平均管线耗时 | 654s |

## 分类明细

| 分类 | 次数 | 聚类OK | 编排OK | 平均覆盖率 | 问题数 |
|------|------|--------|--------|------------|--------|
| AI前沿 | 6 | 5/6 | 5/6 | 92% | 7 |
| 其他 | 3 | 3/3 | 3/3 | 82% | 3 |
| 商业财经 | 6 | 4/6 | 5/6 | 79% | 12 |
| 国际时政 | 7 | 6/7 | 6/7 | 89% | 9 |
| 影音文娱 | 4 | 4/4 | 4/4 | 85% | 3 |
| 技术产业 | 4 | 4/4 | 3/4 | 79% | 4 |
| 游戏电竞 | 6 | 4/6 | 6/6 | 90% | 3 |
| 生命健康 | 5 | 4/5 | 4/5 | 84% | 7 |
| 生活杂谈 | 4 | 3/4 | 3/4 | 74% | 5 |
| 硬件数码 | 3 | 2/3 | 2/3 | 79% | 5 |
| 科学探索 | 3 | 2/3 | 3/3 | 69% | 5 |

## 问题 Trace 清单

### `trace_20260202_1518_商业财经_98336627.md`
- 分类: 商业财经
- VERBATIM_COPY: 3 条新闻被逐字复制

### `trace_20260202_1528_国际时政_873be27f.md`
- 分类: 国际时政
- LOW_COVERAGE: 仅 77% 的素材被写入播报稿

### `trace_20260202_2005_影音文娱_984db38b.md`
- 分类: 影音文娱
- VERBATIM_COPY: 1 条新闻被逐字复制

### `trace_20260202_2024_生命健康_2900c631.md`
- 分类: 生命健康
- VERBATIM_COPY: 2 条新闻被逐字复制

### `trace_20260202_2038_技术产业_3d738c89.md`
- 分类: 技术产业
- LOW_COVERAGE: 仅 79% 的素材被写入播报稿

### `trace_20260202_2047_国际时政_724d87fa.md`
- 分类: 国际时政
- VERBATIM_COPY: 3 条新闻被逐字复制

### `trace_20260203_0813_科学探索_5b8ff7a7.md`
- 分类: 科学探索
- LOW_COVERAGE: 仅 67% 的素材被写入播报稿
- VERBATIM_COPY: 1 条新闻被逐字复制

### `trace_20260203_0818_商业财经_5bf4defa.md`
- 分类: 商业财经
- LOW_COVERAGE: 仅 71% 的素材被写入播报稿

### `trace_20260203_0855_生活杂谈_f6545988.md`
- 分类: 生活杂谈
- LOW_COVERAGE: 仅 33% 的素材被写入播报稿

### `trace_20260203_0859_硬件数码_fb84f59c.md`
- 分类: 硬件数码
- LOW_COVERAGE: 仅 67% 的素材被写入播报稿
- VERBATIM_COPY: 1 条新闻被逐字复制

### `trace_20260203_1517_生命健康_4f0daab8.md`
- 分类: 生命健康
- LOW_COVERAGE: 仅 50% 的素材被写入播报稿

### `trace_20260203_1531_商业财经_96c7bd50.md`
- 分类: 商业财经
- LOW_COVERAGE: 仅 59% 的素材被写入播报稿
- VERBATIM_COPY: 2 条新闻被逐字复制

### `trace_20260203_1544_影音文娱_3e85e7eb.md`
- 分类: 影音文娱
- VERBATIM_COPY: 7 条新闻被逐字复制

### `trace_20260203_1550_国际时政_3b63d501.md`
- 分类: 国际时政
- LOW_COVERAGE: 仅 72% 的素材被写入播报稿
- VERBATIM_COPY: 2 条新闻被逐字复制

### `trace_20260203_2013_国际时政_a2da92be.md`
- 分类: 国际时政
- VERBATIM_COPY: 4 条新闻被逐字复制

### `trace_20260204_1507_其他_ca9df3d9.md`
- 分类: 其他
- LOW_COVERAGE: 仅 67% 的素材被写入播报稿

### `trace_20260204_1509_国际时政_ae898a41.md`
- 分类: 国际时政
- VERBATIM_COPY: 3 条新闻被逐字复制

### `trace_20260204_1524_技术产业_547030d1.md`
- 分类: 技术产业
- VERBATIM_COPY: 1 条新闻被逐字复制

### `trace_20260204_1533_生命健康_d65bd50e.md`
- 分类: 生命健康
- VERBATIM_COPY: 2 条新闻被逐字复制

### `trace_20260204_1558_商业财经_e37f937e.md`
- 分类: 商业财经
- LOW_COVERAGE: 仅 77% 的素材被写入播报稿
- VERBATIM_COPY: 1 条新闻被逐字复制
- REPEATED_CONTENT: 发现 2 处重复语句

### `trace_20260204_2003_影音文娱_4796cf71.md`
- 分类: 影音文娱
- LOW_COVERAGE: 仅 62% 的素材被写入播报稿

### `trace_20260205_0031_生活杂谈_73eb0761.md`
- 分类: 生活杂谈
- PLANNING_FAILED: 编排失败 (Error: LLM Connection Failed: error sending request for url (http://127.0.0.1:1234/v1/chat/completions). Fallback to simple grouping.)
- LOW_COVERAGE: 仅 75% 的素材被写入播报稿

### `trace_20260205_0040_技术产业_115bcf8d.md`
- 分类: 技术产业
- PLANNING_FAILED: 编排失败 (Error: LLM Connection Failed: error sending request for url (http://127.0.0.1:1234/v1/chat/completions). Fallback to simple grouping.)
- LOW_COVERAGE: 仅 67% 的素材被写入播报稿

### `trace_20260205_0049_科学探索_ae504229.md`
- 分类: 科学探索
- CLUSTERING_FALLBACK: 聚类全部回退，相关新闻未被正确分组
- LOW_COVERAGE: 仅 60% 的素材被写入播报稿
- VERBATIM_COPY: 1 条新闻被逐字复制

### `trace_20260205_0100_游戏电竞_40d464d7.md`
- 分类: 游戏电竞
- CLUSTERING_FALLBACK: 聚类全部回退，相关新闻未被正确分组
- VERBATIM_COPY: 2 条新闻被逐字复制

### `trace_20260205_0115_AI前沿_4bc28037.md`
- 分类: AI前沿
- VERBATIM_COPY: 4 条新闻被逐字复制

### `trace_20260205_0126_生命健康_450bbef2.md`
- 分类: 生命健康
- VERBATIM_COPY: 5 条新闻被逐字复制

### `trace_20260205_0725_商业财经_edda0ae7.md`
- 分类: 商业财经
- CLUSTERING_FALLBACK: 聚类全部回退，相关新闻未被正确分组
- VERBATIM_COPY: 1 条新闻被逐字复制

### `trace_20260205_0738_游戏电竞_01d52884.md`
- 分类: 游戏电竞
- CLUSTERING_FALLBACK: 聚类全部回退，相关新闻未被正确分组

### `trace_20260205_0751_硬件数码_a9350d60.md`
- 分类: 硬件数码
- CLUSTERING_FALLBACK: 聚类全部回退，相关新闻未被正确分组
- PLANNING_FAILED: 编排失败 (Error: LLM Connection Failed: error sending request for url (http://127.0.0.1:1234/v1/chat/completions). Fallback to simple grouping.)
- VERBATIM_COPY: 1 条新闻被逐字复制

### `trace_20260205_2114_AI前沿_ddfe6012.md`
- 分类: AI前沿
- CLUSTERING_FALLBACK: 聚类全部回退，相关新闻未被正确分组
- HIGH_COPY_PASTE: 平均 89% 的内容直接复制摘要
- VERBATIM_COPY: 12 条新闻被逐字复制

### `trace_20260205_2129_商业财经_7a04e89e.md`
- 分类: 商业财经
- CLUSTERING_FALLBACK: 聚类全部回退，相关新闻未被正确分组
- PLANNING_FAILED: 编排失败 (Error: LLM Connection Failed: error sending request for url (http://127.0.0.1:1234/v1/chat/completions). Fallback to simple grouping.)
- VERBATIM_COPY: 15 条新闻被逐字复制

### `trace_20260208_1557_生活杂谈_543f65aa.md`
- 分类: 生活杂谈
- CLUSTERING_FALLBACK: 聚类全部回退，相关新闻未被正确分组
- VERBATIM_COPY: 4 条新闻被逐字复制

### `trace_20260208_2019_AI前沿_643ed7ba.md`
- 分类: AI前沿
- PLANNING_FAILED: 编排失败 (Error: LLM Connection Failed: error sending request for url (https://llm.hackerlife.fun:8443/v1/chat/completions). Fallback to simple grouping.)
- HIGH_COPY_PASTE: 平均 100% 的内容直接复制摘要
- VERBATIM_COPY: 11 条新闻被逐字复制

### `trace_20260208_2032_国际时政_5dd87eca.md`
- 分类: 国际时政
- CLUSTERING_FALLBACK: 聚类全部回退，相关新闻未被正确分组
- PLANNING_FAILED: 编排失败 (Error: LLM Connection Failed: error sending request for url (https://llm.hackerlife.fun:8443/v1/chat/completions). Fallback to simple grouping.)
- VERBATIM_COPY: 11 条新闻被逐字复制

### `trace_20260208_2047_其他_e12d7c08.md`
- 分类: 其他
- HIGH_COPY_PASTE: 平均 100% 的内容直接复制摘要
- VERBATIM_COPY: 3 条新闻被逐字复制

### `trace_20260208_2053_生命健康_1109cdf1.md`
- 分类: 生命健康
- CLUSTERING_FALLBACK: 聚类全部回退，相关新闻未被正确分组
- PLANNING_FAILED: 编排失败 (Error: LLM Connection Failed: error sending request for url (https://llm.hackerlife.fun:8443/v1/chat/completions). Fallback to simple grouping.)
- VERBATIM_COPY: 5 条新闻被逐字复制
