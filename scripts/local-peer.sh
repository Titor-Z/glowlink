#!/usr/bin/env bash
# 在同一台机器上起一个「真对端」守护进程，用来在桌面版里看到**真实设备**
# （预览假数据 `GLOWLINK_PREVIEW=1` 只用于看排版，不参与任何协议）。
#
# 为什么必须独立 HOME：身份、信任列表、状态文件都按 `$HOME/.glowlink` 存。
# 同一个 HOME 起第二个守护进程会共用同一份身份 → 发现层按 ID 去重，
# 你会看到「只有自己」；独立 HOME 才是两台设备。
#
# 用法：
#   ./scripts/local-peer.sh                 # 起对端（后台，端口沿用默认 9267）
#   ./scripts/local-peer.sh --port 9330     # 换发现端口（自己那份也要一起换）
#   ./scripts/local-peer.sh --stop          # 停掉
#   ./scripts/local-peer.sh --status
#
# 起来之后：
#   1. 桌面版/CLI 会自动把它当成一台真设备；
#   2. 想让它接收，用 `--yes`（免人工确认，已信任才免；首次仍需配对）；
#   3. 收下的文件在 `$HOME/.glowlink-peer/received`。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PEER_HOME="${GLOWLINK_PEER_HOME:-$HOME/.glowlink-peer}"
PORT="9267"
ACTION="start"
EXTRA=()

while [ $# -gt 0 ]; do
  case "$1" in
    --stop) ACTION="stop" ;;
    --status) ACTION="status" ;;
    --port) PORT="$2"; shift ;;
    --yes) EXTRA+=("--yes") ;;
    *) echo "未知参数：$1" >&2; exit 2 ;;
  esac
  shift
done

BIN="$ROOT/target/debug/glowlink"
STATE="$PEER_HOME/.glowlink/state.json"
PIDFILE="$PEER_HOME/daemon.pid"

# 真实进程号只有**状态文件**里有：`glowlink on` 会把 `__daemon` 派生出去后自己退出，
# 于是 `$!` 拿到的是启动器（早就没了），照它去 kill 只会留下孤儿进程越堆越多。
peer_pid() {
  [ -f "$STATE" ] || return 1
  grep -o '"pid" *: *[0-9]*' "$STATE" | grep -o '[0-9]*' | head -1
}

# 本机那份守护进程**绝不**碰：只停自己起的这个。
stop_peer() {
  local pid had=0
  if pid="$(peer_pid)" && kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    for _ in $(seq 1 20); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.15
    done
    kill -9 "$pid" 2>/dev/null || true
    echo "已停掉对端守护进程（PID ${pid}）"
    had=1
  fi
  rm -f "$PIDFILE" "$STATE"
  [ "$had" = 1 ] || echo "没有在跑的对端守护进程"
}

case "$ACTION" in
  stop)
    stop_peer
    exit 0
    ;;
  status)
    if [ -f "$STATE" ]; then
      echo "对端在跑：$PEER_HOME"
      cat "$STATE"
    else
      echo "对端没在跑（$STATE 不存在）"
    fi
    exit 0
    ;;
esac

if [ ! -x "$BIN" ]; then
  echo "先编译：cargo build -p glowlink-cli --bin glowlink" >&2
  exit 1
fi
# 门禁看「状态文件里的 pid 还活着吗」，不看文件存不存在：
# 被 kill 掉时状态文件会残留（CLI 只在正常退出时删它），拿存在性当门禁会让你再也起不来。
if pid="$(peer_pid)" && kill -0 "$pid" 2>/dev/null; then
  echo "对端已经在跑了（PID ${pid}，先 --stop）" >&2
  exit 1
fi
rm -f "$PIDFILE" "$STATE"  # 清掉上一次的残留

mkdir -p "$PEER_HOME/received"
# 默认带 --yes：这是一台只用来收文件的替身，**首次**配对仍会要求人工确认
# （`--yes` 只对已信任指纹免确认，这是 Step 3 定下的边界，不改）。
if [ ${#EXTRA[@]} -eq 0 ]; then EXTRA+=("--yes"); fi

HOME="$PEER_HOME" nohup "$BIN" on --discovery-port "$PORT" \
  --dir "$PEER_HOME/received" "${EXTRA[@]}" \
  >"$PEER_HOME/daemon.out" 2>&1 &

for _ in $(seq 1 50); do
  [ -f "$STATE" ] && break
  sleep 0.2
done

if [ -f "$STATE" ]; then
  echo "对端守护进程已启动（PID $(peer_pid)，HOME=${PEER_HOME}，发现端口 ${PORT}）"
  echo "  设备名与前一台相同（都是这台机器）——界面里靠指纹区分"
  echo "  收件目录：$PEER_HOME/received"
  echo "  停掉：$0 --stop"
else
  echo "启动失败，看 $PEER_HOME/daemon.out" >&2
  exit 1
fi
