#!/bin/sh
# Run a throwaway xherd daemon with fake agents, for screenshots.
#   scripts/demo.sh [dir]     (default /tmp/xherd-demo; UI at http://127.0.0.1:4741/)
# Uses its own data root, workspace and HOME, and stub `claude`/`gh` commands, so
# nothing of yours is touched. Needs a built daemon (cargo build) and ui/dist.
# Ctrl-C stops it. Agents are all parked/paused/watching/done: a "running" one
# without a live tmux session would be failed at start.
set -eu
dir=$(cd "$(dirname "${1:-/tmp/xherd-demo}")" 2>/dev/null && pwd)/$(basename "${1:-/tmp/xherd-demo}")
here=$(cd "$(dirname "$0")" && pwd)
xherd="${XHERD_BIN:-$here/../target/debug/xherd}"
port=4741
rm -rf "$dir"; mkdir -p "$dir/home" "$dir/ws" "$dir/fakehome" "$dir/bin"
printf '#!/bin/sh\necho "claude 2.1.319"\n' > "$dir/bin/claude"
printf '#!/bin/sh\necho "gh version 2.60.0"\n' > "$dir/bin/gh"
chmod +x "$dir/bin/claude" "$dir/bin/gh"
printf '{"port":%s,"workspace":"%s/ws","welcomed":true,"tmuxSocket":"xherd-demo"}' "$port" "$dir" > "$dir/home/config.json"
run() { PATH="$dir/bin:$PATH" HOME="$dir/fakehome" XHERD_HOME="$dir/home" exec "$xherd" daemon; }
# first start creates the database, then seed it
run >/dev/null 2>&1 & pid=$!
sleep 4; kill $pid; wait $pid 2>/dev/null || true
python3 "$here/demo-seed.py" "$dir/home" "$dir/ws"
token=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['userToken'])" "$dir/home/config.json")
(sleep 4; curl -s -X POST "http://127.0.0.1:$port/api/v1/install" -H "authorization: Bearer $token" \
  -H 'content-type: application/json' -d '{}' >/dev/null) &
echo "xherd demo on http://127.0.0.1:$port/ (data in $dir)"
run
