FreshLoop (鲜阅) - 分布式架构设计文档1. 项目概述FreshLoop 是一个完全私有化、不依赖任何外部云服务的现代化个人阅读工具。它采用分布式架构，利用本地运行的大模型（Local LLM）和本地语音合成引擎，将互联网上的新鲜资讯转化为结构化的“禅意”阅读体验。核心理念：计算与存储分离 (Decoupled)：核心计算节点 (Cortex) 与数据节点 (Nexus) 逻辑分离，既可单机部署也可跨网络部署。静态配置驱动 (Config-as-Code)：通过配置文件管理任务源与系统参数，易于版本控制与备份。轻量与连接 (Link First)：只存储元数据与必要的生成内容（如 TTS 音频），源站大文件（图片/视频）直接使用外链。隐私与自主 (Local Intelligence)：全本地化 AI 处理流程。2. 系统架构 (System Architecture)系统划分为服务端 (Server Side) 和 工作节点 (Worker Side) 两大逻辑域。graph TD
    User((我))
    
    subgraph "Server Side (Data & Presentation)"
        Gateway[Reverse Proxy]
        Web[FreshLoop Web]
        Nexus[Nexus Service (API & DB Owner)]
        DB[(SQLite DB File)]
        FS_Audio[Local Audio Storage]
    end

    subgraph "Worker Side (Compute Node)"
        ConfigFile["config.toml (RSS/Prompt)"]
        Cortex[Cortex Service (Crawler/LLM/TTS)]
        
        LocalLLM[Ollama]
        LocalTTS[Piper TTS]
    end

    %% 用户访问链路
    User --HTTPS--> Gateway
    Gateway --/api--> Nexus
    Gateway --/audio--> FS_Audio
    Gateway --/*--> Web

    %% 数据持久化
    Nexus <--> DB

    %% 任务执行链路
    ConfigFile -.-> Cortex
    Cortex --1. Fetch--> Internet((Internet))
    Cortex --2. Inference--> LocalLLM
    Cortex --3. Synthesize--> LocalTTS
    
    %% 结果回传
    Cortex --4. POST /api/internal/items (Auth)--> Nexus
    Cortex --5. Upload Audio--> FS_Audio
3. 功能模块详解3.1 前端：Web Client (Port: 3000)技术栈：Next.js, Tailwind CSS核心功能：沉浸式阅读：纯粹的内容展示界面，通过 GET /api/items 获取时间流信息。Zen Mode：极简音频播放模式，提供无干扰的听书体验。状态展示：仅展示内容，不包含任何系统配置或管理入口。3.2 枢纽服务：Nexus Service (Port: 8080)角色：数据状态管理者 (State Manager)。职责：数据库独占：唯一拥有 SQLite 数据库读写权限的服务。API 接口：GET /api/items：面向前端，提供分页内容查询。POST /api/internal/items：面向 Cortex，接收处理完成的结构化数据（需 X-NEXUS-KEY 鉴权）。POST /api/internal/upload：面向 Cortex，接收生成的音频文件流。3.3 核心服务：Cortex Service (Daemon)角色：计算工作者 (Compute Worker)。职责：守护进程：作为后台服务运行，由配置文件驱动任务调度。任务循环：加载配置：启动时读取 config.toml 获取 RSS 源和模型参数。采集与处理：执行爬虫抓取、调用 Ollama 进行清洗与摘要、调用 Piper 生成语音。数据上报：将生成的 Metadata 和音频文件通过 HTTP 接口推送到 Nexus。4. 数据与存储设计4.1 部署目录结构Server Side (Nexus 节点)/opt/freshloop/server/
├── data/
│   ├── freshloop.db  # SQLite 数据库文件
│   └── audio/        # 静态音频文件存储目录
└── nexus_app         # 服务端二进制文件
Worker Side (Cortex 节点)/opt/freshloop/worker/
├── config.toml       # 核心配置文件
└── cortex_app        # 工作端二进制文件
4.2 配置文件 (config.toml)Cortex 通过此文件管理所有业务逻辑配置。[nexus]
api_url = "[http://192.168.1.10:8080](http://192.168.1.10:8080)"  # Nexus 服务地址
auth_key = "my-secret-key-123"        # 通信鉴权密钥

[llm]
model = "llama3"
api_url = "http://localhost:11434"

[tts]
model_path = "./zh_CN-huayan-medium.onnx"

[[sources]]
name = "Hacker News"
url = "[https://news.ycombinator.com/rss](https://news.ycombinator.com/rss)"
interval_min = 60
tags = ["Tech", "Global"]

[[sources]]
name = "少数派"
url = "[https://sspai.com/feed](https://sspai.com/feed)"
interval_min = 120
tags = ["Life", "Digital"]
4.3 数据库模型 (SQLite)仅包含核心内容数据。Items 表 Schema字段名类型说明idTEXT (PK)UUIDtitleTEXT标题summaryTEXTLLM 生成的摘要original_urlTEXT原文跳转链接cover_image_urlTEXT源站图片外链audio_urlTEXT指向 Nexus 托管的音频文件路径publish_timeINTEGER发布时间戳created_atINTEGER入库时间戳5. 核心工作流5.1 自动化处理链路任务触发：Cortex 根据 config.toml 中的 interval_min 触发特定源的更新任务。智能处理：Fetch：抓取 HTML/RSS 内容。Think：发送文本至本地 Ollama 接口生成摘要。Speak：发送摘要至本地 Piper 进程生成 MP3 音频。数据同步：Cortex 调用 Nexus 的上传接口发送音频文件。Cortex 组装 JSON 数据（含外链、摘要、音频路径），携带 Auth Key 推送至 Nexus 数据库接口。前端展现：用户刷新 Web 页面，Nexus 返回最新的 Item 列表。5.2 配置管理链路修改：用户直接编辑 Cortex 节点的 config.toml 文件（如添加新源）。生效：重启 Cortex 服务 (systemctl restart freshloop-cortex)，新配置即刻生效。6. 技术栈总结模块角色技术选型说明NexusState ManagerRust (Axum)高并发 API 服务，负责数据持久化CortexCompute WorkerRust (Tokio)异步任务运行时，负责业务逻辑与 AI 编排ConfigConfigurationTOML静态配置文件标准DatabaseStorageSQLite单文件数据库，易于备份与迁移
