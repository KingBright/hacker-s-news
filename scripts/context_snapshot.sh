#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

section() {
    printf '\n== %s ==\n' "$1"
}

section "FreshLoop Context Snapshot"
date "+%Y-%m-%d %H:%M:%S %Z"
printf 'Root: %s\n' "$ROOT"

section "Git"
printf 'Branch: '
git branch --show-current 2>/dev/null || true
printf 'HEAD: '
git rev-parse --short HEAD 2>/dev/null || true
printf '\nChanged files:\n'
git status --short 2>/dev/null | sed -n '1,120p' || true

section "Must Read"
for file in \
    AGENTS.md \
    task.md \
    docs/agent-memory.md \
    docs/build-and-deploy.md \
    docs/freshloop-product-style.md
do
    if [ -f "$file" ]; then
        printf '[ok] %s\n' "$file"
    else
        printf '[missing] %s\n' "$file"
    fi
done

section "Canonical Commands"
printf '%s\n' \
    'Context:   ./scripts/context_snapshot.sh' \
    'Backend:   cd backend && cargo test -p cortex && cargo test -p nexus' \
    'Frontend:  cd frontend && npm run lint && npm run build' \
    'Android:   cd android_client && flutter analyze' \
    'APK:       ./scripts/deploy.sh --android' \
    'Deploy:    ./scripts/deploy.sh'

section "Pending Checklist"
if [ -f task.md ]; then
    if ! grep -n '^- \[ \]' task.md; then
        printf 'No unchecked task.md items.\n'
    fi
else
    printf 'task.md is missing.\n'
fi

section "Recent Lessons"
for file in "$HOME/.happy_coding/knowledge/tooling.md" "$HOME/.happy_coding/knowledge/process.md"; do
    if [ -f "$file" ]; then
        printf '\n-- %s --\n' "$file"
        tail -80 "$file"
    fi
done

section "Deploy Script Notes"
if [ -f scripts/deploy.sh ]; then
    rg -n 'openjdk|JAVA_HOME|flutter build|DEPLOY_ANDROID|Preparing remote directories' scripts/deploy.sh || true
fi
