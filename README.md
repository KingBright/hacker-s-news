# FreshLoop (Hacker's News) 📻

**FreshLoop** 是一个由 AI 驱动的个性化新闻电台应用，致力于提供完全私有化、自动化的新闻聚合与播报体验。该项目的核心设计理念是**低算力需求**与**高质量输出**的平衡，让用户仅需消费级硬件即可运行整套智能广播系统。

## 🏗️ 核心架构 (3-Tier)

项目采用清晰的三层架构设计，各司其职：

1.  **Frontend (UI)**
    *   基于 **Next.js** + **TailwindCSS** 构建。
    *   提供现代化的 Web 播放器，支持 PWA（渐进式 Web 应用），完美适配移动端（支持锁屏播放、后台控制）。

2.  **Nexus (Backend)**
    *   使用 **Rust (Axum)** 编写的高性能后端服务。
    *   负责核心业务数据的存储 (SQLite)、API 接口暴露以及系统的鉴权管理。

3.  **Cortex (Intelligent Core)**
    *   **Rust** 编写的智能中枢。
    *   系统的"大脑"，负责全自动化的业务流程：RSS 抓取 -> 内容去重 -> LLM 摘要 -> 编排 TTS 生成 -> 音频上传。

## 🧩 AI 能力集成

为了实现真人级的新闻播报体验，我们深度集成了以下 AI 能力：

### 🗣️ TTS 推理 (Aha + VoxCPM)
我们使用了 **[Aha](https://github.com/KingBright/aha)** 作为大模型推理框架（Fork 自 [jhqxxx/aha](https://github.com/jhqxxx/aha)，特别感谢原作者的杰出工作）。
*   **模型**: 搭载 **VoxCPM** 模型，提供极具表现力的中文语音合成。
*   **优势**: 这是一个轻量级的推理框架，经过优化后可在较低配置的硬件上高效运行。

### 🧠 LLM (大语言模型)
*   Cortex 能够对接任何兼容 **OpenAI API** 格式的大模型（如 Nvidia Nemotron, Llama 3, GPT-4o 等）。
*   主要用于新闻内容的分类、清洗、摘要生成以及口语化润色。

## 🚀 快速开始

需要强调的是，由于本人日常使用 mac 作为开发机，因此并本系统主要被设计在 mac 上运行和部署。当然实际上代码并没有太多平台依赖，可以很方便地迁移到其他平台（如果有兴趣欢迎 fork 与共建）。

### 1. 配置
复制 `config.toml.example` 为 `config.toml`，填入你的 RSS 源 API Key 等信息。

### 2. 一键部署 Cortex 服务
使用提供的脚本编译并注册为系统服务（macOS）：
```bash
./scripts/install_local_service.sh
```

### 3. 一键部署前端和 Nexus 服务
```bash
./scripts/deploy.sh
```

打开浏览器访问 `http://localhost:3000`，开始您的个性化新闻之旅。

## 🤝 贡献
欢迎提交 Pull Request 或 Issue！
