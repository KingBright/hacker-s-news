---
name: colab-operator
description: 使用 Google Colab CLI 在云端调度高性能 GPU/TPU (如 L4 24G, A100) 运行高负载 Python 任务，如 IndexTTS 2.5/VoxCPM 语音克隆、YuE 2 音乐生成与批量模型推理。支持 Google Drive 持久化环境/模型缓存、防 WebSocket 断连心跳、自动管理云端实例生命周期，跑完即停，避免浪费 CU 算力。
---

# Colab Cloud Operator (Google Colab 算力调度技能)

本技能用于将高负载的 AI 推理与批处理任务（如 IndexTTS 2.5 / VoxCPM2 语音克隆、YuE 2 音乐生成、批量模型推理等）从本地 Mac 卸载至 Google Colab 的云端 GPU/TPU 实例（如 NVIDIA L4 24GB、A100 等），保护本地 Mac GPU 资源并获得实时或超实时生成速度。

---

## 核心原则与安全防线

> [!CAUTION]
> **绝对防止 CU 算力空耗：**
> Colab 实例一旦处于运行（Connected）状态，每小时都会按固定速率扣除 Compute Units（L4 约 1.54 CU/h）。
> - **任务结束或脚本异常退出后，必须无条件在 `finally` 块或收尾步骤执行 `colab stop -s <name>` 销毁实例！**
> - 每次任务前后必须执行 `colab sessions` 确认 `Active assignments: 0`。

> [!IMPORTANT]
> **1. 本地网络代理要求：**
> 在本机调用 `colab` 命令与 Google 服务通信时，必须携带本地代理环境变量（端口 8228）：
> `https_proxy=http://127.0.0.1:8228 http_proxy=http://127.0.0.1:8228 /Users/jinliang/.local/bin/colab <command>`
>
> **2. 防止 WebSocket 空闲断连（关键）：**
> `colab exec` 通过代理建立 Jupyter Kernel WebSocket 连接。若远端耗时命令（如 `uv sync`、下载模型、长批量推理）超过 20~30 秒无标准输出，代理可能断开空闲连接并报 `RuntimeError: Connection was lost`。
> - 远端执行耗时子进程时，**必须使用 `run_cmd_heartbeat`（每 5 秒输出一次进度心跳）**。
> - 文件传输一律使用基于 HTTPS REST (`/api/contents/`) 的 `colab upload` 与 `colab download`，不要通过 `colab exec` 管道传输大文件。
>
> **3. 非阻塞 Google Drive 挂载：**
> 原生 `colab drivemount` 在首次未授权时会尝试打开 `/dev/tty` 等待回车，导致 Agent 后台任务永久挂起。应使用非阻塞凭证传播脚本检测 `dfs_ephemeral` 状态，挂载 `/content/drive/MyDrive/FreshLoop_Colab_Cache` 实现模型与虚拟环境秒级恢复。

---

## 常用操作指引

### 1. 查询算力余量与活跃会话
```bash
# 查询当前剩余 Compute Units 与每小时消耗率
https_proxy=http://127.0.0.1:8228 http_proxy=http://127.0.0.1:8228 /Users/jinliang/.local/bin/colab usage

# 查询当前活跃运行的会话列表
https_proxy=http://127.0.0.1:8228 http_proxy=http://127.0.0.1:8228 /Users/jinliang/.local/bin/colab sessions
```

### 2. 标准多步骤批处理生命周期 (`colab new` -> `upload` -> `exec -f` -> `download` -> `stop`)
```bash
# 1. 创建命名 L4 GPU 会话
https_proxy=http://127.0.0.1:8228 http_proxy=http://127.0.0.1:8228 /Users/jinliang/.local/bin/colab new -s indextts-l4 --gpu L4

# 2. 上传参考音频或任务清单 (走 HTTPS REST，极速稳定)
https_proxy=http://127.0.0.1:8228 http_proxy=http://127.0.0.1:8228 /Users/jinliang/.local/bin/colab upload -s indextts-l4 /path/to/vivian.wav vivian.wav

# 3. 执行带心跳保护的远端脚本 (--timeout 600)
https_proxy=http://127.0.0.1:8228 http_proxy=http://127.0.0.1:8228 /Users/jinliang/.local/bin/colab exec -s indextts-l4 -f examples/indextts_example.py --timeout 600

# 4. 下载生成结果回本地 Mac
https_proxy=http://127.0.0.1:8228 http_proxy=http://127.0.0.1:8228 /Users/jinliang/.local/bin/colab download -s indextts-l4 indextts25_output.wav /local/path/output.wav

# 5. 【铁律】任务完成立即释放 L4 实例
https_proxy=http://127.0.0.1:8228 http_proxy=http://127.0.0.1:8228 /Users/jinliang/.local/bin/colab stop -s indextts-l4
```

---

## IndexTTS 2.5 云端部署与缓存恢复规范（重要）

1. **Python 版本隔离（严禁直接使用 Colab 默认 Python 3.13）**：
   - Colab 默认 Kernel 为 Python 3.13，而官方 `index-tts` (`pyproject.toml`) 严格要求 `requires-python = ">=3.10,<3.12"`。
   - **正确做法**：克隆官方仓库 `https://github.com/index-tts/index-tts.git` 至 `/content/index-tts`，使用 Colab 预装的 `/usr/local/bin/uv sync --python 3.11` 自动创建独立的 Python 3.11 虚拟环境 (`/content/index-tts/.venv`) 并安装 CUDA 12.8 版 PyTorch 2.8 与全部依赖（请勿添加 `--extra deepspeed`，避免无意义的编译耗时）。
2. **模型权重获取与 Google Drive 缓存**：
   - 官方权重位于 HuggingFace：`IndexTeam/IndexTTS-2.5`（含 `config.yaml`, `gpt.pth`, `s2mel.pth`, `codec.pth`, `feat1.pt`, `feat2.pt`, `wav2vec2bert_stats.pt`, `multilingual_zh_ja_yue_char_del.tiktoken`）。
   - 若已挂载 `/content/drive/MyDrive/FreshLoop_Colab_Cache/indextts25_checkpoints.tar`，优先从 Google Drive 解压恢复；否则通过 `huggingface_hub.snapshot_download("IndexTeam/IndexTTS-2.5", local_dir="/content/index-tts/checkpoints")` 拉取（Colab 内部千兆网络约 20~30 秒），并在首次拉取后同步归档至 Google Drive。
3. **官方推理调用方式 (`IndexTTS-2.5`)**：
   ```python
   import sys
   sys.path.insert(0, "/content/index-tts")
   from indextts.infer_v2_5 import IndexTTS2

   tts = IndexTTS2(
       cfg_path="/content/index-tts/checkpoints/config.yaml",
       model_dir="/content/index-tts/checkpoints",
       use_bf16=True,
   )
   tts.infer(
       spk_audio_prompt="/content/vivian.wav",
       text="欢迎收听 FreshLoop 新闻速递。",
       output_path="/content/output.wav",
       verbose=False,
   )
   ```

---

## 双模型云端混排与 A/B 轮换规范 (`VoxCPM2` + `IndexTTS 2.5`)

统一云端执行器 [`examples/cloud_tts_runner.py`](file:///Users/jinliang/.gemini/config/skills/colab-operator/examples/cloud_tts_runner.py) 与本地编排器 [`scripts/colab_tts_client.py`](file:///Users/jinliang/.gemini/config/skills/colab-operator/scripts/colab_tts_client.py) 同时部署了 **VoxCPM2** (`openbmb/VoxCPM2`) 与 **IndexTTS 2.5** (`IndexTeam/IndexTTS-2.5`)：

1. **A/B 定时轮换策略 (`--engine auto`)**：
   - **早间 Radio (`morning-program`, 00:00–13:59 CST)**：使用 **`voxcpm2`**（`openbmb/VoxCPM2` Ultimate Cloning 模式，英文技术术语发音精准，L4 RTF ~0.3–0.4）。
   - **晚间 Radio (`evening-program`, 14:00–23:59 CST)**：使用 **`indextts25`**（`IndexTeam/IndexTTS-2.5`，情感韵律丰富，自动将四位年份 `2026年` 归一化为 `二零二六年` 并对大写英文缩略词做字母级分词保护）。
   - **周报 (`curated_weekly` / `weekly_digest`)**：使用 **`indextts25`**。
   - **精读文章 (`reading_article`)**：按 `job_id` SHA256 哈希奇偶性 50/50 确定性分流至 **`voxcpm2`** 与 **`indextts25`**（重试时保持模型一致）。
2. **音频质量增强与队列复用**：
   - 自动按中文句末标点将长稿切分为 `<= 150~160` 字安全片段，逐块检查时长与 RMS，异常块自动重试。
   - 统一去除直流偏置（DC offset）、施加 15ms 余弦淡入 / 25ms 余弦淡出消除拼接爆音，并归一化到广播级响度（Target RMS `0.085`，峰值软限幅 `-1.0 dBFS`），输出 24kHz 16-bit 单声道 PCM WAV 与 128kbps MP3。
   - 多任务并发入队时通过 `/Users/jinliang/.freshloop/cache/colab-tts.lock` 与 `.waiting` 队列标记自动复用热启动的 Colab L4 会话；当队列清空时，在 `finally` 块立即执行 `colab stop` 释放实例，确保零算力空耗。

