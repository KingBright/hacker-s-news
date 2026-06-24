#!/usr/bin/env python3
import sys
from pathlib import Path

# /// script
# dependencies = [
#   "torch",
#   "safetensors",
#   "packaging",
#   "numpy",
# ]
# ///

def main():
    if len(sys.argv) < 2:
        print("用法: python3 convert_magictts.py <magictts_36k.pt 的路径> [输出目录]")
        print("例如: uv run convert_magictts.py ~/.aha/SCUT/MAGIC-TTS/magictts_36k.pt ~/.aha/SCUT/MAGIC-TTS")
        sys.exit(1)

    ckpt_path = Path(sys.argv[1])
    if not ckpt_path.exists():
        print(f"错误: 找不到输入权重文件 {ckpt_path}")
        sys.exit(1)

    out_dir = Path(sys.argv[2]) if len(sys.argv) > 2 else ckpt_path.parent
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / "consolidated.safetensors"

    print(f"正在载入 PyTorch 权重: {ckpt_path} ...")
    import torch
    from safetensors.torch import save_file

    state_dict = torch.load(ckpt_path, map_location="cpu")

    # 提取核心的 model state dict
    if "model" in state_dict:
        state_dict = state_dict["model"]
    elif "state_dict" in state_dict:
        state_dict = state_dict["state_dict"]
    elif "model_state_dict" in state_dict:
        state_dict = state_dict["model_state_dict"]

    # 规范化键名 (有些 PyTorch 保存时会带 module. 前缀)
    new_state_dict = {}
    for k, v in state_dict.items():
        name = k
        if k.startswith("module."):
            name = k[7:]
        # 我们底层 C 侧只需要这两个主要计算层进行映射，其它可以用零填充
        new_state_dict[name] = v

    print(f"正在保存为 Safetensors: {out_path} ...")
    save_file(new_state_dict, out_path)
    print("转换成功完成！")

if __name__ == "__main__":
    main()
