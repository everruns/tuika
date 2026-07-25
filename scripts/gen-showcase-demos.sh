#!/usr/bin/env bash
#
# Regenerate the showcase recordings in docs/showcases/.
#
# Unlike the component demos, these record *other people's applications* — the
# hosts listed in docs/showcases.md — so the source of truth is each project's
# own repository. This script clones (or updates) them into a cache directory,
# builds them, drives them with a deterministic offline fixture, and records the
# result with VHS. Nothing here reaches the network at record time beyond the
# clones, and no API key is involved: both scenes are powered by a local LLMSim.
#
# Scenes:
#   yolop   — the agent TUI answering a question, backed by a scripted LLMSim so
#             the transcript is identical on every run.
#   llmsim  — the stats dashboard under live traffic, with error injection on.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), which needs ttyd and
# ffmpeg on PATH, plus git, cargo, and curl.
#
#   scripts/gen-showcase-demos.sh              # both scenes
#   scripts/gen-showcase-demos.sh llmsim       # just this one
#
# Checkouts and build artifacts are cached between runs (a full yolop build is
# slow); override the location with TUIKA_SHOWCASE_CACHE.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cache="${TUIKA_SHOWCASE_CACHE:-${XDG_CACHE_HOME:-${HOME}/.cache}/tuika-showcases}"
out="${repo_root}/docs/showcases"
run="${cache}/run"

# Ports for the two LLMSim instances: one replays the scripted agent session,
# one serves the dashboard scene's traffic.
script_port="${TUIKA_SHOWCASE_SCRIPT_PORT:-8098}"
stats_port="${TUIKA_SHOWCASE_STATS_PORT:-8099}"

for tool in vhs git cargo curl; do
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "error: ${tool} not found on PATH" >&2
    exit 1
  fi
done

scenes=("$@")
if [[ ${#scenes[@]} -eq 0 ]]; then
  scenes=(yolop llmsim)
fi

mkdir -p "${out}" "${cache}/src" "${run}"

# Background processes started per scene; killed on exit so a failed run does
# not leave a simulator or a traffic loop behind.
pids=()
cleanup() {
  for pid in "${pids[@]:-}"; do
    [[ -n "${pid}" ]] && kill "${pid}" 2>/dev/null || true
  done
}
trap cleanup EXIT

# Clone on first run, fast-forward to the remote default branch afterwards.
sync_repo() {
  local url="$1" dir="$2"
  if [[ -d "${dir}/.git" ]]; then
    echo "Updating ${dir}…"
    git -C "${dir}" fetch --quiet --depth 1 origin HEAD
    git -C "${dir}" checkout --quiet --force FETCH_HEAD
  else
    echo "Cloning ${url}…"
    git clone --quiet --depth 1 "${url}" "${dir}"
  fi
}

llmsim_bin="${cache}/src/llmsim/target/debug/llmsim"
yolop_bin="${cache}/src/yolop/target/debug/yolop"

build_llmsim() {
  sync_repo https://github.com/chaliy/llmsim "${cache}/src/llmsim"
  echo "Building llmsim (--features tui)…"
  cargo build --quiet --manifest-path "${cache}/src/llmsim/Cargo.toml" --features tui
}

# ---------------------------------------------------------------------------
# Scene: yolop
#
# A scripted LLMSim stands in for the model, so the recording shows a real
# agent session — tool call, streamed markdown, highlighted code — without a
# provider key and without run-to-run drift. The workspace is a clone of this
# repository, so the answer is about tuika's own examples.
# ---------------------------------------------------------------------------

record_yolop() {
  build_llmsim
  sync_repo https://github.com/everruns/yolop "${cache}/src/yolop"
  echo "Building yolop…"
  cargo build --quiet --manifest-path "${cache}/src/yolop/Cargo.toml"

  # The recorded workspace: a clone of this repository at its current commit.
  local ws="${cache}/tuika"
  if [[ -d "${ws}/.git" ]]; then
    git -C "${ws}" fetch --quiet origin HEAD
    git -C "${ws}" checkout --quiet --force FETCH_HEAD
  else
    git clone --quiet "${repo_root}" "${ws}"
  fi

  # The scripted session. Turn one calls a tool (`list_directory` runs in-process,
  # so the scene needs no shell sandbox); turn two is the answer yolop renders
  # through tuika's markdown and code-block components.
  cat >"${run}/yolop-session.json" <<'EOF'
{
  "on_exhausted": "repeat_last",
  "turns": [
    {
      "type": "tool_calls",
      "calls": [
        { "name": "list_directory", "arguments": { "path": "examples" } }
      ]
    },
    {
      "type": "assistant",
      "text": "Three examples ship with the crate, and each one boots a real terminal UI:\n\n```bash\ncargo run --example gallery    # motion components + OSC 9;4 progress\ncargo run --example markdown   # streaming markdown + highlighted code\ncargo run --example image      # graphics protocols + alt-text fallback\n```\n\n`gallery` is the best starting point — it renders every motion component on one screen and drives the terminal's native progress indicator.\n"
    }
  ]
}
EOF
  cat >"${run}/llmsim-script.toml" <<EOF
# Loopback only: llmsim defaults to 0.0.0.0, and a recording has no reason to
# expose the simulator beyond this machine.
[server]
host = "127.0.0.1"
port = ${script_port}

[latency]
profile = "gpt5"

[response]
script_path = "${run}/yolop-session.json"
EOF

  echo "Starting scripted llmsim on :${script_port}…"
  "${llmsim_bin}" serve --config "${run}/llmsim-script.toml" >"${run}/llmsim-script.log" 2>&1 &
  pids+=($!)
  wait_for_port "${script_port}"

  # Captured at the component gallery's pixel density — ~19 px per cell against a
  # displayed width="880", i.e. more than 2x — so a showcase is as crisp as a demo.
  # A host needs a real terminal, though, so the grid is ~103x32 cells rather than
  # the gallery's ~70 columns: yolop's footer and the dashboard's panel grid do not
  # fit in less. That already puts the frame at ~2.8 Mpx; going further (FontSize
  # 40 at this grid) outruns what VHS can capture per second, and the recording
  # then plays back faster than the session really ran. The theme background
  # matches tuika's so VHS's padding blends into the app.
  cat >"${run}/yolop.tape" <<EOF
Output "${out}/yolop.gif"

Set Shell bash
Set FontSize 32
Set Width 2080
Set Height 1370
Set Padding 32
Set WindowBar Colorful
Set Theme { "background": "#141214", "foreground": "#ebe6e6" }
Set Framerate 12

Hide
Type "cd ${ws}"
Enter
Type "clear && CUSTOM_BASE_URL=http://127.0.0.1:${script_port}/openai/v1 CUSTOM_API_KEY=sim TERM=xterm-256color ${yolop_bin} --provider custom -m gpt-5"
Enter
Sleep 5s
Show
Sleep 800ms
Type "how do I run the examples?"
Sleep 700ms
Enter
Sleep 13s
EOF

  echo "Recording docs/showcases/yolop.gif…"
  vhs "${run}/yolop.tape"
}

# ---------------------------------------------------------------------------
# Scene: llmsim
#
# The dashboard is the server, so the recorded process is `llmsim serve --tui`
# and the traffic has to come from outside the recording: a loop of streaming
# and non-streaming requests across four models, against a config with error
# injection on so the Errors panel is not a wall of zeroes.
# ---------------------------------------------------------------------------

record_llmsim() {
  build_llmsim

  cat >"${run}/llmsim-stats.toml" <<EOF
# Loopback only: llmsim defaults to 0.0.0.0, and a recording has no reason to
# expose the simulator beyond this machine.
[server]
host = "127.0.0.1"
port = ${stats_port}

[latency]
profile = "gpt5"

[response]
generator = "lorem"
target_tokens = 160

[errors]
rate_limit_rate = 0.05
server_error_rate = 0.02
EOF

  cat >"${run}/traffic.sh" <<EOF
#!/usr/bin/env bash
# Drive representative traffic at the recorded dashboard.
base="http://127.0.0.1:${stats_port}"
models=(gpt-5 gpt-4o claude-opus-4-5 gemini-2.5-flash)
i=0
while :; do
  curl -s -o /dev/null -X POST "\${base}/openai/v1/chat/completions" \\
    -H 'content-type: application/json' \\
    -d "{\\"model\\":\\"\${models[\$((i % 4))]}\\",\\"messages\\":[{\\"role\\":\\"user\\",\\"content\\":\\"summarize the build\\"}],\\"stream\\":true}" &
  curl -s -o /dev/null -X POST "\${base}/anthropic/v1/messages" \\
    -H 'content-type: application/json' \\
    -d '{"model":"claude-opus-4-5","max_tokens":128,"messages":[{"role":"user","content":"hello"}]}' &
  i=\$((i + 1))
  sleep 0.35
done
EOF
  chmod +x "${run}/traffic.sh"

  cat >"${run}/llmsim.tape" <<EOF
Output "${out}/llmsim.gif"

Set Shell bash
Set FontSize 32
Set Width 2080
Set Height 1370
Set Padding 32
Set WindowBar Colorful
Set Theme { "background": "#141214", "foreground": "#ebe6e6" }
Set Framerate 10

Hide
Type "clear && TERM=xterm-256color ${llmsim_bin} serve --config ${run}/llmsim-stats.toml --tui"
Enter
Sleep 3s
Show
Sleep 12s
EOF

  # The loop starts before the server exists; those first requests just fail.
  echo "Starting traffic against :${stats_port}…"
  "${run}/traffic.sh" >/dev/null 2>&1 &
  pids+=($!)

  echo "Recording docs/showcases/llmsim.gif…"
  vhs "${run}/llmsim.tape"
}

wait_for_port() {
  local port="$1"
  for _ in $(seq 1 60); do
    if curl -s -o /dev/null "http://127.0.0.1:${port}/llmsim/stats"; then
      return 0
    fi
    sleep 0.5
  done
  echo "error: llmsim did not come up on :${port}" >&2
  exit 1
}

for scene in "${scenes[@]}"; do
  case "${scene}" in
    yolop) record_yolop ;;
    llmsim) record_llmsim ;;
    *)
      echo "error: unknown scene \`${scene}\` (known: yolop, llmsim)" >&2
      exit 1
      ;;
  esac
  cleanup
  pids=()
done

echo "Done. Wrote:"
for scene in "${scenes[@]}"; do
  printf '  %s (%s)\n' "docs/showcases/${scene}.gif" \
    "$(du -h "${out}/${scene}.gif" | cut -f1)"
done
