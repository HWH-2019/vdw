#!/usr/bin/env bash
# open-workspace.sh — Detect available editors and open a workspace directory
# Usage: bash scripts/open-workspace.sh <workspace_path>

WORKSPACE_PATH="$1"

if [ -z "$WORKSPACE_PATH" ] || [ ! -d "$WORKSPACE_PATH" ]; then
  echo "Usage: bash open-workspace.sh <workspace_path>"
  echo "Error: workspace path does not exist"
  exit 1
fi

echo "Workspace: $WORKSPACE_PATH"

# Priority order — try each editor in sequence
# The first available editor with working path detection wins

# 1. Qoder (current IDE)
if [ -d "/Applications/Qoder.app" ]; then
  echo "Found: Qoder"
  open -a Qoder "$WORKSPACE_PATH"
  exit 0
fi

# 2. VS Code
if command -v code &>/dev/null; then
  echo "Found: VS Code"
  code "$WORKSPACE_PATH"
  exit 0
elif [ -d "/Applications/Visual Studio Code.app" ]; then
  echo "Found: VS Code"
  open -a "Visual Studio Code" "$WORKSPACE_PATH"
  exit 0
fi

# 3. Cursor
if command -v cursor &>/dev/null; then
  echo "Found: Cursor"
  cursor "$WORKSPACE_PATH"
  exit 0
elif [ -d "/Applications/Cursor.app" ]; then
  echo "Found: Cursor"
  open -a Cursor "$WORKSPACE_PATH"
  exit 0
fi

# 4. Trae
if command -v trae &>/dev/null; then
  echo "Found: Trae"
  trae "$WORKSPACE_PATH"
  exit 0
elif [ -d "/Applications/Trae.app" ]; then
  echo "Found: Trae"
  open -a Trae "$WORKSPACE_PATH"
  exit 0
fi

# 5. Windsurf
if command -v windsurf &>/dev/null; then
  echo "Found: Windsurf"
  windsurf "$WORKSPACE_PATH"
  exit 0
elif [ -d "/Applications/Windsurf.app" ]; then
  echo "Found: Windsurf"
  open -a Windsurf "$WORKSPACE_PATH"
  exit 0
fi

# Fallback: open in Finder
echo "No known editor found. Opening in Finder..."
open "$WORKSPACE_PATH"
