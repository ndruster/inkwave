#!/usr/bin/env node
// TR-16.2: wasm smoke test over Chrome DevTools Protocol (no npm deps;
// requires Node >= 22 for the global WebSocket).
//
// Prerequisite: a chromium-family browser exposing CDP, e.g.
//   rm -rf /tmp/smoke-profile
//   chromium-browser --headless=new --no-sandbox --disable-gpu \
//       --disable-dev-shm-usage \
//       --user-data-dir=/tmp/smoke-profile \
//       --remote-debugging-port=9222 --window-size=1920,1080 about:blank &
// and the wasm build served over http (trunk serve, or any static server
// pointed at rust/dist).
//
//   --user-data-dir must be a FRESH directory: the persistent default profile
//   replays a stale cached index.html/wasm across runs. --disable-dev-shm-usage
//   avoids GPU-process SIGSEGVs on containers with a small /dev/shm (Docker
//   default 64-350 MB) — environmental noise, not an application signal.
//
// Usage:
//   node rust/tools/verify/wasm-smoke.mjs <page-url> [seconds] [cdp-port]
//
// What it does:
//   1. attaches to the browser's startup about:blank target (the foreground
//      tab; a background tab's canvas.getContext() returns null in headless)
//      and injects a requestAnimationFrame frame counter before load;
//   2. navigates to <page-url>, waits for boot, presses Enter (menu -> match,
//      "开局"), and re-presses it every second while sampling (idempotent:
//      Enter is a no-op during the HUD);
//   3. collects application errors: uncaught exceptions, console messages of
//      level "error", AND wgpu/Bevy rendering failures, which on wasm are
//      logged via tracing as console type "log" with a "%cERROR%c" style
//      prefix — matching only type==="error" would silently miss them;
//   4. prints a JSON summary {frames, fps_mean, fps_p50, fps_semantics,
//      panicked, rendering_errors, error_count, errors[]} and exits non-zero
//      on any error/panic/no frames.
//
// FPS caveat: fps_* measure rAF tick rate (fps_semantics:"raf_tick"), not
// per-frame render throughput of the game. On a software renderer the app may
// die early (shader compile failure) while this counter keeps ticking; the
// rendering_errors/panicked fields are what gate TR-16.2, not the FPS number.
const url = process.argv[2];
const seconds = Number(process.argv[3] ?? 60);
const port = process.argv[4] ?? "9222";
if (!url) {
  console.error("usage: wasm-smoke.mjs <page-url> [seconds] [cdp-port]");
  process.exit(2);
}

const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const target = list.find((t) => t.type === "page" && t.url === "about:blank");
if (!target) {
  // No fallback to an arbitrary page: re-using a target that already loaded
  // the game leaves the previous wasm instance holding the canvas WebGL2
  // context and produces a spurious panic. One smoke per browser instance.
  console.error("no about:blank page target on CDP port", port, "- start a fresh browser");
  process.exit(2);
}

const ws = new WebSocket(target.webSocketDebuggerUrl);
let wsDead = null;
ws.onclose = (e) => {
  wsDead = "websocket closed: " + (e.reason || "code " + e.code);
  // Fail fast: in-flight sends would otherwise hang forever.
  for (const { rej } of pending.values()) rej(new Error(wsDead));
  pending.clear();
};
ws.onerror = () => {
  wsDead = "websocket error";
};
await Promise.race([
  new Promise((res, rej) => {
    // addEventListener (not the on* property) so the wsDead sentinels above
    // are not overwritten by the connect race.
    ws.addEventListener("open", res);
    ws.addEventListener("error", rej);
  }),
  new Promise((_, rej) => setTimeout(() => rej(new Error("CDP connect timeout")), 10000)),
]).catch((e) => {
  console.error("cannot attach to CDP target:", String(e.message ?? e));
  process.exit(2);
});

let id = 0;
const pending = new Map();
const errors = [];
let panicked = false;
// wgpu/Bevy failures that reach the page as console type "log" (see header).
const RENDER_ERR_RE =
  /Shader compilation failed|Shader translation error|Caught rendering error|Quitting the application|panicked at|RuntimeError: unreachable/i;
function send(method, params = {}) {
  return new Promise((res, rej) => {
    const mid = ++id;
    pending.set(mid, { res, rej });
    ws.send(JSON.stringify({ id: mid, method, params }));
  });
}
ws.onmessage = (ev) => {
  const msg = JSON.parse(ev.data);
  if (msg.id && pending.has(msg.id)) {
    const { res, rej } = pending.get(msg.id);
    pending.delete(msg.id);
    if (msg.error) rej(new Error(`CDP ${msg.error.message} (method ${msg.method ?? "?"})`));
    else res(msg.result);
    return;
  }
  if (msg.method === "Runtime.exceptionThrown") {
    const d = msg.params.exceptionDetails;
    const text = d.exception?.description ?? d.text;
    errors.push("exception: " + String(text).split("\n").slice(0, 6).join(" | "));
    if (String(text).includes("panicked") || String(text).includes("unreachable")) {
      panicked = true;
    }
  }
  if (msg.method === "Runtime.consoleAPICalled") {
    const args = (msg.params.args ?? [])
      .map((a) => String(a.value ?? a.description ?? a.unserializableValue ?? ""))
      .join(" ");
    if (msg.params.type === "error") {
      errors.push("console.error: " + args.split("\n").slice(0, 6).join(" | "));
    } else if (RENDER_ERR_RE.test(args)) {
      // tracing logs Bevy/wgpu ERROR level as console "log" on wasm.
      errors.push("render-error: " + args.split("\n").slice(0, 4).join(" | "));
      if (/panicked at|unreachable/i.test(args)) panicked = true;
    }
  }
  if (msg.method === "Log.entryAdded" && msg.params.entry.level === "error") {
    const e = msg.params.entry;
    // The browser auto-requests /favicon.ico; a 404 there is not an
    // application error (trunk build emits no favicon in M1).
    if (String(e.url || "").endsWith("/favicon.ico")) return;
    errors.push("log: " + String(e.text).split("\n")[0] + " [" + String(e.url || "") + "]");
  }
};

try {
  await run();
} catch (e) {
  console.error("smoke harness failure:", String(e.message ?? e));
  process.exit(3);
}

async function run() {
  await send("Runtime.enable");
  await send("Log.enable");
  await send("Page.enable");
  await send("Network.enable");
  await send("Network.setCacheDisabled", { cacheDisabled: true });
  // Frame counter injected before any script of the game page runs.
  await send("Page.addScriptToEvaluateOnNewDocument", {
    source:
      "window.__smoke = {frames: 0, t0: performance.now()};" +
      "(function loop(){ window.__smoke.frames++; requestAnimationFrame(loop); })();",
  });
  await send("Page.navigate", { url });

  // "开局": after boot settles, press Enter (menu -> HUD shortcut, Task 13).
  await new Promise((r) => setTimeout(r, 6000));
  const pressEnter = () =>
    (async () => {
      for (const type of ["keyDown", "keyUp"]) {
        await send("Input.dispatchKeyEvent", {
          type,
          key: "Enter",
          code: "Enter",
          windowsVirtualKeyCode: 13,
          nativeVirtualKeyCode: 13,
        });
      }
    })();
  await pressEnter();

  const deadline = Date.now() + seconds * 1000;
  // Per-second frame deltas -> a real p50 of the rAF tick rate; re-pressing
  // Enter covers slow boots (44 MB wasm) where the first press raced the menu.
  const perSec = [];
  let lastFrames = 0;
  while (Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 1000));
    if (wsDead) throw new Error(wsDead);
    await pressEnter();
    const p = await send("Runtime.evaluate", {
      expression: "String(window.__smoke?.frames ?? -1)",
      returnByValue: true,
    });
    const f = Number(p.result.value);
    if (f >= 0) perSec.push(f - lastFrames);
    lastFrames = f;
  }

  const probe = await send("Runtime.evaluate", {
    expression:
      "JSON.stringify({frames: window.__smoke?.frames ?? -1, elapsed_ms: performance.now() - (window.__smoke?.t0 ?? 0)})",
    returnByValue: true,
  });
  const { frames, elapsed_ms } = JSON.parse(probe.result.value);
  const fps = elapsed_ms > 0 ? (frames / elapsed_ms) * 1000 : 0;
  const sorted = [...perSec].sort((a, b) => a - b);
  const fps_p50 = sorted.length ? sorted[Math.round((sorted.length - 1) * 0.5)] : 0;

  const dedup = [...new Set(errors)];
  const renderErrCount = dedup.filter((e) => e.startsWith("render-error:")).length;
  console.log(
    JSON.stringify(
      {
        url,
        seconds,
        frames,
        fps_mean: Number(fps.toFixed(2)),
        fps_p50,
        fps_semantics: "raf_tick",
        panicked,
        rendering_errors: renderErrCount,
        error_count: dedup.length,
        errors: dedup.slice(0, 20),
      },
      null,
      2,
    ),
  );
  process.exit(dedup.length > 0 || panicked || frames <= 0 ? 1 : 0);
}
