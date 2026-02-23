<script>
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";

  const NUM_BARS = 12;
  let phase = $state("init");
  let processing = $state(false);
  let statusMsg = $state("Loading...");
  let downloadMsg = $state("");
  let accessWarning = $state(false);
  let accessHint = $state(false);
  let bars = $state(Array(NUM_BARS).fill(0.05));
  let hovered = $state(false);

  let hintTimer;
  let hintText = $state("");
  let hintVisible = $state(false);
  let statsText = $state("");
  let statsVisible = $state(false);

  let pillColor = $state("#12121e");
  let pillOpacity = $state(0.44);

  function hexToRgba(hex, alpha) {
    const r = parseInt(hex.slice(1,3), 16);
    const g = parseInt(hex.slice(3,5), 16);
    const b = parseInt(hex.slice(5,7), 16);
    return `rgba(${r}, ${g}, ${b}, ${alpha})`;
  }

  function pillBg() { return hexToRgba(pillColor, pillOpacity); }

  function parseRgba(s) {
    const m = s.match(/rgba?\((\d+),\s*(\d+),\s*(\d+),?\s*([\d.]*)\)/);
    if (m) {
      pillColor = "#" + [m[1],m[2],m[3]].map(x => parseInt(x).toString(16).padStart(2,"0")).join("");
      pillOpacity = m[4] ? parseFloat(m[4]) : 1;
    }
  }

  async function loadPillColor() {
    try {
      const saved = await invoke("get_pill_color");
      if (saved) parseRgba(saved);
    } catch {}
  }

  function startHintPolling() {
    clearInterval(hintTimer);
    hintText = "";
    hintVisible = false;
    hintTimer = setInterval(async () => {
      if (phase !== "ready" || processing || accessWarning || accessHint) return;
      try {
        const h = await invoke("get_hint");
        if (h && h !== hintText) {
          hintVisible = false;
          setTimeout(() => { hintText = h; hintVisible = true; }, 300);
        }
      } catch {}
    }, 20000); // rotate every 20s
    // Also fire immediately
    setTimeout(async () => {
      if (phase !== "ready" || processing) return;
      try {
        const h = await invoke("get_hint");
        if (h) { hintText = h; hintVisible = true; }
      } catch {}
    }, 2000);
  }

  function clearHint() {
    hintText = "";
    hintVisible = false;
    clearInterval(hintTimer);
  }

  let rawLevel = 0;
  let smoothBars = Array(NUM_BARS).fill(0.05);

  function updateBars(level) {
    rawLevel = level;
    const v = Math.min(level * 10, 1);
    for (let i = 0; i < NUM_BARS; i++) {
      const center = (NUM_BARS - 1) / 2;
      const dist = Math.abs(i - center) / center;
      const envelope = 1 - dist * 0.6;
      const noise = 0.7 + Math.random() * 0.6;
      const target = Math.max(0.06, v * envelope * noise);
      smoothBars[i] += (target > smoothBars[i] ? 0.6 : 0.15) * (target - smoothBars[i]);
    }
    bars = [...smoothBars];
  }

  let idleFrame;
  function idleAnimate() {
    if (phase !== "listening") return;
    if (rawLevel < 0.005) {
      for (let i = 0; i < NUM_BARS; i++) {
        const t = Date.now() / 800 + i * 0.5;
        smoothBars[i] += 0.1 * (0.06 + Math.sin(t) * 0.03 - smoothBars[i]);
      }
      bars = [...smoothBars];
    }
    idleFrame = requestAnimationFrame(idleAnimate);
  }

  let savePosTimer;
  async function onMoved() {
    clearTimeout(savePosTimer);
    savePosTimer = setTimeout(async () => {
      try {
        const pos = await getCurrentWindow().outerPosition();
        await invoke("save_window_pos", { x: pos.x, y: pos.y });
      } catch {}
    }, 500);
  }

  async function startListening() {
    if (phase !== "ready") return;
    await invoke("start_listening");
    phase = "listening";
    statusMsg = "Listening";
    clearHint();
    idleAnimate();
  }

  async function stopListening() {
    if (phase !== "listening") return;
    await invoke("stop_listening");
    phase = "ready";
    statusMsg = "Ready";
    smoothBars.fill(0.05);
    bars = [...smoothBars];
    cancelAnimationFrame(idleFrame);
    startHintPolling();
  }

  async function toggle() {
    if (phase === "listening") await stopListening();
    else if (phase === "ready") await startListening();
  }

  async function closeWidget() {
    await stopListening();
    await getCurrentWindow().hide();
  }

  async function showWidget() {
    await getCurrentWindow().show();
  }

  onMount(async () => {
    const win = getCurrentWindow();

    try {
      const saved = await invoke("get_window_pos");
      if (saved.x && saved.y) {
        await win.setPosition({ type: "Physical", x: parseInt(saved.x), y: parseInt(saved.y) });
      } else {
        const monitor = await win.currentMonitor();
        if (monitor) {
          const wx = Math.round((monitor.size.width - 300) / 2);
          const wy = monitor.size.height - 120;
          await win.setPosition({ type: "Physical", x: wx, y: wy });
        }
      }
    } catch {}

    await win.onMoved(onMoved);
    await loadPillColor();

    document.addEventListener("pointerleave", () => { hovered = false; });

    await listen("audio_level", (e) => updateBars(e.payload));
    await listen("pipeline_state", (e) => { processing = e.payload === "processing"; });
    await listen("dictation_stats", (e) => {
      const { words, seconds } = e.payload;
      if (words > 0) {
        statsText = `${words} words in ${seconds}s`;
        statsVisible = true;
        setTimeout(() => { statsVisible = false; statsText = ""; }, 3500);
      }
    });
    await listen("accessibility_missing", () => { if (!accessHint) accessWarning = true; });
    await listen("accessibility_granted", () => { accessWarning = false; accessHint = false; });

    // Fallback: poll to clear stuck accessibility hint
    setInterval(async () => {
      if (!accessHint && !accessWarning) return;
      try {
        const granted = await invoke("check_accessibility_cmd");
        if (granted) { accessWarning = false; accessHint = false; }
      } catch {}
    }, 3000);

    await listen("download_progress", (e) => {
      const p = e.payload;
      const pct = p.total > 0 ? Math.round(p.downloaded / p.total * 100) : 0;
      const mb = (p.downloaded / 1048576).toFixed(0);
      const totalMb = (p.total / 1048576).toFixed(0);
      downloadMsg = `${p.model} ${mb}/${totalMb} MB (${pct}%)`;
      statusMsg = downloadMsg;
    });

    // Normal mode: toggle listening
    await listen("toggle_listening", async () => {
      await showWidget();
      if (phase === "ready") await startListening();
      else if (phase === "listening") await stopListening();
    });

    // Walkie-talkie mode: hold to record, release to process
    await listen("walkie_press", async () => {
      await showWidget();
      if (phase === "ready") await startListening();
    });
    await listen("walkie_release", async () => {
      if (phase === "listening") await stopListening();
    });

    await listen("show_window", async () => { await showWidget(); });
    await listen("pill_color_changed", (e) => { parseRgba(e.payload); });

    await listen("mic_changed", async () => {
      if (phase === "listening") {
        await stopListening();
        await startListening();
      }
    });

    // Auto-init: download if needed, then load
    try {
      const ms = await invoke("check_models");
      if (!ms.all_ready) {
        phase = "loading";
        statusMsg = "Downloading models...";
        await invoke("download_models");
      }
      phase = "loading";
      statusMsg = "Loading models...";
      await invoke("load_models");
      phase = "ready";
      statusMsg = "Ready";
      startHintPolling();
    } catch (e) { statusMsg = `Error: ${e}`; }
  });
</script>

<main>
  <div class="pill" class:listening={phase === "listening"} class:processing
    style="background: {pillBg()};"
    onpointerenter={() => hovered = true}
    onpointerleave={() => hovered = false}
    onmousedown={(e) => { if (e.target.closest('button') || e.target.closest('.settings-panel')) return; getCurrentWindow().startDragging(); }}>

    <button class="mic-btn" onclick={toggle} disabled={phase === "init" || phase === "loading"}>
      {#if phase === "listening"}
        <div class="wave">
          {#each bars as h}
            <div class="bar" style="height:{Math.max(3, h * 28)}px"></div>
          {/each}
        </div>
      {:else if phase === "loading"}
        <div class="spinner"></div>
      {:else}
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
          <path d="M12 1a4 4 0 0 1 4 4v6a4 4 0 0 1-8 0V5a4 4 0 0 1 4-4z"/>
          <path d="M19 10v1a7 7 0 0 1-14 0v-1"/><line x1="12" y1="19" x2="12" y2="23"/>
        </svg>
      {/if}
    </button>

    {#if processing}<div class="proc-dot"></div>{/if}

    <span class="label">
      {#if accessHint && phase === "ready"}<span class="access-hint">Find OpenFlow in the list → toggle on</span>{:else if accessWarning && phase === "ready"}<span class="access-link" onclick={() => { invoke("open_accessibility_settings"); accessWarning = false; accessHint = true; }}>⚠ Enable Accessibility →</span>{:else if processing}Processing{:else if statsVisible && statsText}<span class="hint hint-visible">{statsText}</span>{:else if hintText && hintVisible && phase === "ready"}<span class="hint" class:hint-visible={hintVisible}>{hintText}</span>{:else}{statusMsg}{/if}
    </span>

    {#if hovered}
      <button class="close-btn" onclick={closeWidget}>
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round">
          <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
        </svg>
      </button>
    {/if}
  </div>
</main>

<style>
  :global(*) { box-sizing: border-box; }
  :global(html), :global(body) {
    margin: 0; padding: 0; background: transparent !important;
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", sans-serif;
    overflow: hidden;
  }
  main {
    display: flex; flex-direction: column; align-items: center;
    padding: 0;
  }
  .pill {
    display: flex; align-items: center; gap: 8px;
    padding: 6px 12px 6px 6px;
    background: rgba(18, 18, 30, 0.44);
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
    border: 1px solid rgba(255,255,255,0.07);
    border-radius: 22px;
    box-shadow: none;
    transition: border-color 0.3s, box-shadow 0.3s;
    position: relative;
    cursor: grab;
    user-select: none;
  }
  .pill.listening {
    border-color: rgba(80, 200, 120, 0.3);
    box-shadow: 0 2px 20px rgba(0,0,0,0.45), 0 0 12px rgba(80,200,120,0.08);
  }
  .pill.processing {
    border-color: rgba(100, 140, 255, 0.35);
    box-shadow: 0 2px 20px rgba(0,0,0,0.45), 0 0 12px rgba(100,140,255,0.1);
  }
  .mic-btn {
    width: 32px; height: 32px; border-radius: 50%;
    border: none; cursor: pointer;
    background: rgba(255,255,255,0.05);
    color: #888; display: flex; align-items: center; justify-content: center;
    transition: all 0.2s; flex-shrink: 0;
  }
  .mic-btn:hover { background: rgba(255,255,255,0.1); color: #ccc; }
  .mic-btn:disabled { opacity: 0.35; cursor: default; }
  .pill.listening .mic-btn { background: rgba(80,200,120,0.12); color: #50c878; }
  .wave { display: flex; align-items: center; gap: 1.5px; height: 28px; }
  .bar {
    width: 2.5px; border-radius: 1.5px;
    background: linear-gradient(to top, #3a9d5c, #50c878);
    transition: height 0.08s ease-out;
    min-height: 3px;
  }
  .proc-dot {
    width: 5px; height: 5px; border-radius: 50%;
    background: #648cff; flex-shrink: 0;
    animation: blink 0.7s ease-in-out infinite alternate;
  }
  @keyframes blink { from{opacity:0.25} to{opacity:1} }
  .spinner {
    width: 14px; height: 14px;
    border: 2px solid rgba(255,255,255,0.08);
    border-top-color: #777;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .label {
    color: #777; font-size: 11px; white-space: nowrap;
    letter-spacing: 0.01em; overflow: hidden; text-overflow: ellipsis; max-width: 200px;
    cursor: default;
  }
  .pill.listening .label { color: #50c878; }
  .pill.processing .label { color: #648cff; }
  .close-btn {
    width: 18px; height: 18px; border-radius: 50%;
    border: none; cursor: pointer;
    background: rgba(255,255,255,0.06);
    color: #666; display: flex; align-items: center; justify-content: center;
    transition: all 0.15s; flex-shrink: 0;
  }
  .close-btn:hover { background: rgba(255,80,80,0.25); color: #ff6666; }
  .access-link { cursor: pointer; color: #e8a838; }
  .access-link:hover { color: #f0c060; text-decoration: underline; }
  .access-hint { color: #888; font-style: italic; }
  .hint { color: #666; font-style: italic; opacity: 0; transition: opacity 0.5s ease; }
  .hint-visible { opacity: 1; }
</style>
