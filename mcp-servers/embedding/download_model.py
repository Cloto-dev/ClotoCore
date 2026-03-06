"""Download all-MiniLM-L6-v2 ONNX model for local embedding generation."""

import os
import sys
import urllib.request

MODEL_DIR = os.environ.get("ONNX_MODEL_DIR", "data/models/all-MiniLM-L6-v2")
BASE_URL = os.environ.get(
    "ONNX_MODEL_BASE_URL",
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main",
)

FILES = {
    "model.onnx": f"{BASE_URL}/onnx/model.onnx",
    "tokenizer.json": f"{BASE_URL}/tokenizer.json",
}


def download():
    os.makedirs(MODEL_DIR, exist_ok=True)

    for filename, url in FILES.items():
        dest = os.path.join(MODEL_DIR, filename)
        if os.path.exists(dest):
            print(f"  Already exists: {dest}")
            continue

        print(f"  Downloading: {filename} ...")
        try:
            urllib.request.urlretrieve(url, dest)
            size_mb = os.path.getsize(dest) / (1024 * 1024)
            print(f"  Saved: {dest} ({size_mb:.1f} MB)")
        except Exception as e:
            print(f"  Failed to download {filename}: {e}", file=sys.stderr)
            if os.path.exists(dest):
                os.remove(dest)
            return False

    print(f"Model ready at {MODEL_DIR}")
    return True


if __name__ == "__main__":
    print("=== Downloading all-MiniLM-L6-v2 ONNX model ===")
    success = download()
    sys.exit(0 if success else 1)
