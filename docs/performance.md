# Performance measurements

These are local, reproducible measurements of the packaged release build, not
guarantees for every Mac. The point is to make the “native and lean” claim
falsifiable instead of comparing marketing numbers.

## Environment

| | |
| --- | --- |
| Build | `0.1.0-beta.6`, release profile |
| Machine | Apple M1, 16 GB RAM |
| OS | macOS 27.0.1 |
| App | `dist/Dunlin.app/Contents/MacOS/Dunlin` |
| Harness | none connected |
| Filesystem | warm, except where marked “first launch” |

## Method

- Launch the packaged app directly with a fresh `DUNLIN_DATA_DIR`.
- Detect the first on-screen window for that PID with
  `CGWindowListCopyWindowInfo`; report wall time from process spawn.
- Take the median of three warm launches.
- Read resident memory and lifetime CPU with `ps` after 8 seconds idle.
- Repeat with one trusted local project and with this repository (1,748 source
  files, excluding build output and dependencies).
- The project fixture is an empty project created by the app; no agent process
  is started and no network request is made during the run.

## Results

| Metric | Empty onboarding | One local project | This repository |
| --- | ---: | ---: | ---: |
| First window, warm (median) | **0.31 s** | **0.30 s** | **0.32 s** |
| First window, first launch after install | 1.07 s | — | — |
| Idle RSS after 8 s | **99 MiB** | **103 MiB** | **106 MiB** |
| Idle CPU after 8 s | **0.4%** | **0.4%** | **0.4%** |

Packaging:

- App executable: **42.4 MiB**
- App bundle: **43 MiB**
- Relay executable: about **9.8 MiB** (release build)

## Interpretation

- The 1.07 s first launch includes cold filesystem/Gatekeeper work; subsequent
  launches are around 0.3 s on this machine.
- Project size changes the first paint only slightly because file, language,
  Git and icon scans are bounded and run off the UI thread.
- Idle CPU is a lifetime average from `ps`, not a sampled profiler. It shows
  there is no busy render loop or Electron/Chromium runtime in the background.
- Memory includes the native UI, GPU-backed rendering and local SQLite state.
  A connected agent, large chat history or graph/trace view will use more.
- These figures were collected from the packaged `0.1.0-beta.6` release build.
  Re-run them on the target Mac if you need release-specific hardware evidence.

## Reproduce

```sh
./scripts/bundle-macos.sh --build
FIX="$(mktemp -d)"
DUNLIN_DATA_DIR="$FIX" \
  DUNLIN_LOG_FILE="$FIX/dunlin.log" \
  dist/Dunlin.app/Contents/MacOS/Dunlin &
PID=$!
# Measure until a window owned by $PID appears, then:
sleep 8
ps -o rss=,%cpu= -p "$PID"
kill "$PID"
```

On macOS, `CGWindowListCopyWindowInfo` (CoreGraphics) gives the first-window
timestamp without screen recording permission. The measurements above used a
small Swift polling helper for that step.
