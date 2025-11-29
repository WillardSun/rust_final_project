#!/usr/bin/env bash
set -euo pipefail

# start_tunnels.sh
# Starts a python static server (port 8000) and two ephemeral cloudflared tunnels
# (one for the chat server on 6142 and one for the static server on 8000),
# updates `index.html` to point `WS_URL` at the chat-server tunnel, and
# prints the created public URLs.
#
# Requirements: `cloudflared`, `python3` (for the static server), `sed`, `grep`.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HTML_FILE="$ROOT_DIR/index.html"
LOGDIR="$ROOT_DIR/.cloudflared-logs"
mkdir -p "$LOGDIR"

PY_PORT=8000
CHAT_PORT=6142

function check_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Error: required command '$1' not found in PATH." >&2
    exit 1
  fi
}

check_cmd cloudflared
check_cmd python3
check_cmd sed
check_cmd grep

echo "Starting python static server on port $PY_PORT (serves $ROOT_DIR)..."
pushd "$ROOT_DIR" >/dev/null
# run simple http server if not already listening on $PY_PORT
if ss -ltn "sport = :$PY_PORT" | grep -q LISTEN; then
  echo "Port $PY_PORT already in use — assuming static server already running."
else
  # start python http.server in background
  nohup python3 -m http.server "$PY_PORT" >"$LOGDIR/python-http.$PY_PORT.log" 2>&1 &
  PY_PID=$!
  echo "Started python http.server (PID=$PY_PID)."
fi
popd >/dev/null

echo "Launching cloudflared tunnel for chat server (port $CHAT_PORT)..."
CF_CHAT_LOG="$LOGDIR/cloudflared-chat.$CHAT_PORT.log"
cloudflared tunnel --url "http://localhost:$CHAT_PORT" >"$CF_CHAT_LOG" 2>&1 &
CF_CHAT_PID=$!

echo "Launching cloudflared tunnel for static server (port $PY_PORT)..."
CF_HTTP_LOG="$LOGDIR/cloudflared-http.$PY_PORT.log"
cloudflared tunnel --url "http://localhost:$PY_PORT" >"$CF_HTTP_LOG" 2>&1 &
CF_HTTP_PID=$!

echo "Waiting for cloudflared to publish public URLs (this may take a few seconds)..."
extract_url() {
  local logfile="$1"
  local url=""
  for i in {1..30}; do
    if url=$(grep -Eo "https?://[A-Za-z0-9._-]+trycloudflare.com" "$logfile" | head -n1); then
      if [[ -n "$url" ]]; then
        echo "$url"
        return 0
      fi
    fi
    sleep 1
  done
  return 1
}

if ! CHAT_URL=$(extract_url "$CF_CHAT_LOG"); then
  echo "Failed to find chat tunnel URL in $CF_CHAT_LOG" >&2
  echo "Cloudflared output (last 100 lines):" >&2
  tail -n 100 "$CF_CHAT_LOG" >&2
  exit 1
fi

if ! HTTP_URL=$(extract_url "$CF_HTTP_LOG"); then
  echo "Failed to find http tunnel URL in $CF_HTTP_LOG" >&2
  echo "Cloudflared output (last 100 lines):" >&2
  tail -n 100 "$CF_HTTP_LOG" >&2
  exit 1
fi

echo "Chat tunnel available at: $CHAT_URL" 
echo "Static site tunnel available at: $HTTP_URL"

# Update index.html WS_URL to use the chat tunnel (wss)
CHAT_HOST=$(echo "$CHAT_URL" | sed -E 's@https?://([^/]+).*@\1@')
NEW_WS="wss://$CHAT_HOST/ws"

if [[ -f "$HTML_FILE" ]]; then
  echo "Updating WS_URL in $HTML_FILE -> $NEW_WS"
  # replace the line that starts with const WS_URL =
  sed -i -E "s@^\s*const WS_URL = .*;@  const WS_URL = \"$NEW_WS\";@" "$HTML_FILE"
else
  echo "Warning: $HTML_FILE not found; skipping index update." >&2
fi

echo
echo "Done. Summary:" 
echo "  - Chat (WS) URL: $CHAT_URL -> index.html WS_URL set to $NEW_WS"
echo "  - Static URL:   $HTTP_URL"
echo
echo "Background process PIDs:" 
echo "  cloudflared (chat) : $CF_CHAT_PID (log: $CF_CHAT_LOG)"
echo "  cloudflared (http) : $CF_HTTP_PID (log: $CF_HTTP_LOG)"
if [[ -n "${PY_PID-}" ]]; then
  echo "  python http.server : $PY_PID"
fi

echo
echo "To stop the background cloudflared processes run:" 
echo "  kill $CF_CHAT_PID $CF_HTTP_PID"
echo "(and kill the python server PID if started)."

echo "You can now open the static tunnel URL in your browser:" 
echo "  $HTTP_URL"

exit 0
