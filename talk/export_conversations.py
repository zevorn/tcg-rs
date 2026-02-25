#!/usr/bin/env python3
"""Export Claude Code conversation JSONL files to Markdown."""

import json
import os
import sys
from datetime import datetime
from pathlib import Path

SRC_DIR = Path.home() / ".claude/projects/-home-zevorn-tcg-rs"
DST_DIR = Path(__file__).parent

def extract_text(content):
    """Extract readable text from message content."""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for item in content:
            if isinstance(item, dict):
                if item.get("type") == "text":
                    parts.append(item.get("text", ""))
                elif item.get("type") == "tool_use":
                    name = item.get("name", "")
                    inp = item.get("input", {})
                    if name == "Write" or name == "Edit":
                        parts.append(f"[Tool: {name} → {inp.get('file_path', '')}]")
                    elif name == "Bash":
                        cmd = inp.get("command", "")
                        parts.append(f"[Tool: Bash → `{cmd[:120]}`]")
                    elif name == "Read":
                        parts.append(f"[Tool: Read → {inp.get('file_path', '')}]")
                    elif name == "Glob":
                        parts.append(f"[Tool: Glob → {inp.get('pattern', '')}]")
                    elif name == "Grep":
                        parts.append(f"[Tool: Grep → {inp.get('pattern', '')}]")
                    else:
                        parts.append(f"[Tool: {name}]")
                elif item.get("type") == "tool_result":
                    pass  # skip tool results for readability
        return "\n".join(parts)
    return str(content)

def clean_system_tags(text):
    """Remove <system-reminder> and other system tags."""
    import re
    text = re.sub(r'<system-reminder>.*?</system-reminder>', '', text, flags=re.DOTALL)
    text = re.sub(r'<local-command-caveat>.*?</local-command-caveat>', '', text, flags=re.DOTALL)
    text = re.sub(r'<local-command-stdout>.*?</local-command-stdout>', '[command output]', text, flags=re.DOTALL)
    text = re.sub(r'<command-name>.*?</command-name>', '', text, flags=re.DOTALL)
    text = re.sub(r'<command-message>.*?</command-message>', '', text, flags=re.DOTALL)
    text = re.sub(r'<command-args>.*?</command-args>', '', text, flags=re.DOTALL)
    return text.strip()

def convert_file(jsonl_path, md_path):
    """Convert a single JSONL conversation to Markdown."""
    messages = []
    first_ts = None
    first_user_msg = ""

    with open(jsonl_path, "r") as f:
        for line in f:
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue

            msg_type = obj.get("type", "")
            if msg_type not in ("user", "assistant"):
                continue

            role = obj.get("message", {}).get("role", msg_type)
            content = obj.get("message", {}).get("content", "")
            ts = obj.get("timestamp", "")

            text = extract_text(content)
            text = clean_system_tags(text)

            if not text.strip():
                continue

            if first_ts is None and ts:
                first_ts = ts
            if role == "user" and not first_user_msg:
                first_user_msg = text[:100].replace("\n", " ")

            messages.append((role, text, ts))

    if not messages:
        return False

    # Write markdown
    session_id = jsonl_path.stem
    date_str = first_ts[:10] if first_ts else "unknown"

    with open(md_path, "w") as f:
        f.write(f"# Conversation {session_id[:8]}\n\n")
        f.write(f"- Date: {date_str}\n")
        f.write(f"- Session: `{session_id}`\n")
        f.write(f"- Messages: {len(messages)}\n\n")
        f.write("---\n\n")

        for role, text, ts in messages:
            time_str = ""
            if ts:
                try:
                    dt = datetime.fromisoformat(ts.replace("Z", "+00:00"))
                    time_str = f" ({dt.strftime('%H:%M')})"
                except:
                    pass

            if role == "user":
                f.write(f"## 🧑 User{time_str}\n\n")
            else:
                f.write(f"## 🤖 Assistant{time_str}\n\n")

            f.write(text.strip() + "\n\n")

    return True

def main():
    jsonl_files = sorted(SRC_DIR.glob("*.jsonl"), key=lambda p: p.stat().st_mtime)
    print(f"Found {len(jsonl_files)} conversation files")

    exported = 0
    for i, jsonl_path in enumerate(jsonl_files):
        # Get date from file mtime for naming
        mtime = datetime.fromtimestamp(jsonl_path.stat().st_mtime)
        date_prefix = mtime.strftime("%Y%m%d")
        session_short = jsonl_path.stem[:8]
        md_name = f"{date_prefix}-{session_short}.md"
        md_path = DST_DIR / md_name

        if convert_file(jsonl_path, md_path):
            size_kb = md_path.stat().st_size / 1024
            print(f"  [{i+1}] {md_name} ({size_kb:.0f} KB)")
            exported += 1

    print(f"\nExported {exported} conversations to {DST_DIR}")

if __name__ == "__main__":
    main()
