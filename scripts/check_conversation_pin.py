#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Verify the published conversation profile locally, without network access."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DIRECTORY = ROOT / "contracts" / "agent-conversation-v1"
LOCK_SHA256 = "6e579a2624c79dc8472951a95c74a8b460b386e396845c75816014b1b6c86e40"
lock_bytes = (DIRECTORY / "lock.json").read_bytes()
if hashlib.sha256(lock_bytes).hexdigest() != LOCK_SHA256:
    raise SystemExit("conversation release lock changed")
lock = json.loads(lock_bytes)
for relative, expected in lock["files"].items():
    target = (DIRECTORY / relative).resolve()
    if not target.is_relative_to(DIRECTORY) or not target.is_file():
        raise SystemExit(f"invalid pinned file: {relative}")
    if hashlib.sha256(target.read_bytes()).hexdigest() != expected:
        raise SystemExit(f"conversation pin mismatch: {relative}")
print(f"verified {len(lock['files'])} published conversation files")
