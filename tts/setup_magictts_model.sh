#!/bin/bash
set -e

# 设置代理 (如果可用)
export http_proxy=http://127.0.0.1:8228
export https_proxy=http://127.0.0.1:8228

MODEL_DIR="${1:-$HOME/.aha/SCUT/MAGIC-TTS}"
mkdir -p "$MODEL_DIR"

echo "=== 1. 开始下载 F5-TTS 基础 Vocab 字典 ==="
curl -L -o "$MODEL_DIR/vocab.txt" https://huggingface.co/SWivid/F5-TTS/raw/main/F5TTS_Base/vocab.txt

echo "=== 2. 开始从 Hugging Face 下载 MAGIC-TTS 5.4GB 权重 ==="
echo "注意：由于国内网络连接限制，已自动启动 8228 本地代理进行高速下载..."
echo "如果连接中断，脚本会自动进行断点续传，直到下载全部完成。"
URL="https://huggingface.co/maimai11/MAGIC-TTS/resolve/main/checkpoints/magictts_36k.pt"
until curl -C - -L -o "$MODEL_DIR/magictts_36k.pt" "$URL"; do
    echo "⚠️ 连接中断，将在 5 秒后自动尝试断点续传..."
    sleep 5
done

echo "=== 3. 正在一键转换权重为 Safetensors 格式 ==="
uv run convert_magictts.py "$MODEL_DIR/magictts_36k.pt" "$MODEL_DIR"

echo "=== 4. 清理临时权重文件 ==="
rm "$MODEL_DIR/magictts_36k.pt"

echo "🎉 MAGIC-TTS 模型下载与 C/Metal 推理格式转换已全部完成！"
echo "权重目录: $MODEL_DIR"
