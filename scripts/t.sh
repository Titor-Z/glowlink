#!/usr/bin/env bash
# 单个测试文件 / 单个测试函数的入口（对应 Jest 的「跑一个文件 / 一个用例」）。
#
# 用法：
#   ./scripts/t.sh <测试文件名> [过滤词…]
#
# 例：
#   ./scripts/t.sh ipc                      跑 tests/ipc.rs 里的全部用例
#   ./scripts/t.sh ipc pending_returns      只跑名字含 pending_returns 的用例
#   ./scripts/t.sh ipc --list               只列用例名，不执行
#   ./scripts/t.sh ipc --exact ping_reports_version_pid_and_ports
#   ./scripts/t.sh transfer                 自动识别属于哪个 crate
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

target="$1"
shift

# 自动识别测试文件属于哪个 crate
if [ -f "glowlink-cli/tests/${target}.rs" ]; then
  package=glowlink-cli
elif [ -f "glowlink-core/tests/${target}.rs" ]; then
  package=glowlink-core
else
  echo "找不到测试文件：glowlink-cli/tests/${target}.rs 或 glowlink-core/tests/${target}.rs" >&2
  echo "现有测试文件：" >&2
  ls glowlink-cli/tests/*.rs glowlink-core/tests/*.rs 2>/dev/null | sed 's#.*/tests/##; s/\.rs$//; s/^/  /' >&2
  exit 1
fi

# 只编译这一个测试目标，并从 cargo 输出里取回二进制路径（比猜 hash 稳）
bin=$(cargo test -p "$package" --test "$target" --no-run -j 2 2>&1 \
  | sed -n 's/.*Executable [^(]*(\(.*\))/\1/p' | tail -1)

if [ -z "${bin}" ] || [ ! -x "${bin}" ]; then
  echo "没能拿到测试二进制，请单独跑一次看错误：" >&2
  echo "  cargo test -p ${package} --test ${target} --no-run" >&2
  exit 1
fi

# 默认单线程：这些用例会真实绑端口/起线程，单线程跑更稳也更好读输出
exec "${bin}" --test-threads=1 "$@"
