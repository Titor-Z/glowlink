#!/usr/bin/env bash
# 单个测试文件 / 单个测试函数的入口（对应 Jest 的「跑一个文件 / 一个用例」）。
#
# 用法：
#   ./scripts/t.sh <测试文件名> [过滤词…]
#
# 例：
#   ./scripts/t.sh ipc                      跑 glowlink-ipc/tests/ipc.rs 里的全部用例
#   ./scripts/t.sh ipc pending_returns      只跑名字含 pending_returns 的用例
#   ./scripts/t.sh ipc --list               只列用例名，不执行
#   ./scripts/t.sh ipc --exact ping_reports_version_pid_and_ports
#   ./scripts/t.sh transfer                 自动识别属于哪个 crate
#
# 注意 desktop：它拖进 wgpu / metal，**首次编译是分钟级**（不是秒级），
# 且 `cargo test -p glowlink-desktop` 会连带编译 GPUI 全套。改 desktop 代码时
# 一次编译完再跑，别反复单文件跑。
#
# 为什么不用 `cargo test --workspace`：那会编译并链接**所有**测试目标，
# 在低配机器上很重。这里只编译指定的那一个目标，之后直接执行二进制，
# 因此「无关改动 + 再跑一次」几乎零成本。
set -euo pipefail

cd "$(dirname "$0")/.."

if [ $# -lt 1 ]; then
  sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
  exit 1
fi

# 支持 `cli:xxx` / `core:xxx` / `ipc:xxx` / `desktop:xxx` 显式指定 crate：
# `config`、`transfer`、`state` 这类名字两个以上 crate 里都有，故意不猜。
spec="$1"
shift
case "${spec}" in
  cli:*)  target="${spec#cli:}";  package=glowlink-cli ;;
  core:*) target="${spec#core:}"; package=glowlink-core ;;
  ipc:*)  target="${spec#ipc:}";  package=glowlink-ipc ;;
  desktop:*) target="${spec#desktop:}"; package=glowlink-desktop ;;
  *:*)
    echo "未知的 crate 前缀：${spec%%:*}（只支持 cli: / core: / ipc: / desktop:）" >&2
    exit 1
    ;;
  *)
    target="${spec}"
    # 按目录数一数这个名字中了几处：中两处就必须显式指定，不替用户猜
    matches=()
    for pkg in glowlink-cli glowlink-core glowlink-ipc glowlink-desktop; do
      [ -f "${pkg}/tests/${target}.rs" ] && matches+=("${pkg}")
    done
    if [ ${#matches[@]} -gt 1 ]; then
      echo "「${target}」在 ${matches[*]} 里都有，请显式指定：" >&2
      for pkg in "${matches[@]}"; do
        case "${pkg}" in
          glowlink-cli) echo "  ./scripts/t.sh cli:${target}" >&2 ;;
          glowlink-core) echo "  ./scripts/t.sh core:${target}" >&2 ;;
          glowlink-ipc) echo "  ./scripts/t.sh ipc:${target}" >&2 ;;
          glowlink-desktop) echo "  ./scripts/t.sh desktop:${target}" >&2 ;;
        esac
      done
      exit 1
    fi
    if [ ${#matches[@]} -eq 1 ]; then
      package="${matches[0]}"
    else
      echo "找不到测试文件：${target}.rs（在 cli / core / ipc / desktop 的 tests/ 里都没找到）" >&2
      echo "现有测试文件：" >&2
      ls glowlink-cli/tests/*.rs glowlink-core/tests/*.rs glowlink-ipc/tests/*.rs glowlink-desktop/tests/*.rs 2>/dev/null \
        | sed 's#.*/tests/##; s/\.rs$//; s/^/  /' >&2
      exit 1
    fi
    ;;
esac

# 只编译这一个测试目标，并从 cargo 输出里取回二进制路径（比猜 hash 稳）。
# 输出先落文件：编译报错时必须原样打出来，不能被管道吞掉（否则表现为「脚本什么都不干」）。
log=$(mktemp)
trap 'rm -f "${log}"' EXIT
if ! cargo test -p "$package" --test "$target" --no-run -j 2 >"${log}" 2>&1; then
  echo "编译失败（${package} / tests/${target}.rs）：" >&2
  cat "${log}" >&2
  exit 1
fi
bin=$(sed -n 's/.*Executable [^(]*(\(.*\))/\1/p' "${log}" | tail -1)
if [ -z "${bin}" ] || [ ! -x "${bin}" ]; then
  echo "没能从 cargo 输出里找到测试二进制，完整输出：" >&2
  cat "${log}" >&2
  exit 1
fi

# 默认单线程：这些用例会真实绑端口/起线程，单线程跑更稳也更好读输出
exec "${bin}" --test-threads=1 "$@"
