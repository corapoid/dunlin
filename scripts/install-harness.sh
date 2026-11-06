#!/usr/bin/env bash
# Fixed recipes, also embedded in the relay. Never execute client-provided shell.
set -euo pipefail
case "${1:-}" in
  opencode) packages=('@opencode/cli@latest') ;;
  codex) packages=('@openai/codex@latest' '@agentclientprotocol/codex-acp@latest') ;;
  claude) packages=('@anthropic-ai/claude-code@latest' '@agentclientprotocol/claude-agent-acp@latest') ;;
  *) echo 'Unknown harness. Choose opencode, codex or claude.' >&2; exit 2 ;;
esac
command -v npm >/dev/null || { echo 'Node.js 22+ and npm are required. Install them on the server, then retry.' >&2; exit 1; }
node -e 'if (+process.versions.node.split(".")[0] < 22) process.exit(1)' || { echo 'Node.js 22+ is required.' >&2; exit 1; }
: "${DUNLIN_TOOLS:?Missing managed tools directory}"
mkdir -p "$DUNLIN_TOOLS"
printf 'Installing %s into the server-managed tools directory…\n' "${packages[*]}"
npm install --global --prefix "$DUNLIN_TOOLS" --registry=https://registry.npmjs.org --userconfig=/dev/null --globalconfig=/dev/null --no-fund --no-audit "${packages[@]}"
printf '\nInstalled. Provider authorization is a separate step.\n'
