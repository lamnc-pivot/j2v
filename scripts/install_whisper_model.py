#!/usr/bin/env python3
"""
Install a faster-whisper model into a target directory.

Usage:
  python3 install_whisper_model.py <model_name> <target_dir>

Output JSON:
  {"success": true, "model_path": "..."}
  {"success": false, "error": "..."}
"""

import json
import os
import sys


def main() -> int:
    if len(sys.argv) < 3:
        print(json.dumps({"success": False, "error": "Usage: install_whisper_model.py <model_name> <target_dir>"}))
        return 1

    model_name = sys.argv[1]
    target_dir = sys.argv[2]

    try:
        os.makedirs(target_dir, exist_ok=True)

        from faster_whisper.utils import download_model

        model_path = download_model(model_name, output_dir=target_dir)

        # Keep marker for backward compatibility with existing flows
        marker = os.path.join(target_dir, ".installed")
        with open(marker, "w", encoding="utf-8"):
            pass

        print(json.dumps({"success": True, "model_path": model_path}))
        return 0
    except Exception as e:
        print(json.dumps({"success": False, "error": str(e)}))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
