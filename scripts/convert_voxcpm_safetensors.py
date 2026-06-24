import sys
import os
import torch
from safetensors.torch import save_file

def main():
    if len(sys.argv) < 2:
        print("Usage: python3 convert_voxcpm_safetensors.py <voxcpm_model_dir>")
        print("Example: python3 convert_voxcpm_safetensors.py ~/.aha/OpenBMB/VoxCPM2")
        sys.exit(1)

    model_dir = os.path.expanduser(sys.argv[1])
    pth_path = os.path.join(model_dir, "audiovae.pth")
    safetensors_path = os.path.join(model_dir, "audiovae.safetensors")

    if not os.path.exists(pth_path):
        print(f"Error: {pth_path} not found.")
        sys.exit(1)

    print(f"Loading {pth_path} ...")
    checkpoint = torch.load(pth_path, map_location="cpu")

    state_dict = checkpoint.get("state_dict", checkpoint)

    # Ensure all tensors are contiguous
    for k, v in state_dict.items():
        if isinstance(v, torch.Tensor):
            state_dict[k] = v.contiguous().cpu()

    print(f"Saving to {safetensors_path} ...")
    save_file(state_dict, safetensors_path)
    print("Success!")

if __name__ == "__main__":
    main()
