/**
 * 系统监控详情弹窗
 */
(function () {
  const POLL_MS = 1000;

  const state = {
    snap: null,
    timer: null,
  };

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function fmtBytes(n) {
    const v = Number(n) || 0;
    if (v < 1024) return `${v} B`;
    const units = ["KB", "MB", "GB", "TB"];
    let x = v / 1024;
    let i = 0;
    while (x >= 1024 && i < units.length - 1) {
      x /= 1024;
      i += 1;
    }
    return `${x.toFixed(x >= 10 ? 0 : 1)} ${units[i]}`;
  }

  function pct(n) {
    if (n == null || !Number.isFinite(n)) return "—";
    return `${Math.round(n)}%`;
  }

  function barClass(p) {
    if (p == null || !Number.isFinite(p)) return "sm-bar";
    if (p >= 90) return "sm-bar is-hot";
    if (p >= 75) return "sm-bar is-warm";
    return "sm-bar";
  }

  function rowBar(label, valueText, p) {
    const w = p != null && Number.isFinite(p) ? Math.max(0, Math.min(100, p)) : 0;
    return `<div class="sm-section">
      <div class="sm-row">
        <span class="sm-row-label">${label}</span>
        <span class="sm-row-value">${valueText}</span>
      </div>
      <div class="${barClass(p)}"><i style="width:${w.toFixed(1)}%"></i></div>
    </div>`;
  }

  function escapeHtml(s) {
    return String(s ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;");
  }

  function render() {
    const app = document.getElementById("app");
    if (!app) return;
    const snap = state.snap;
    if (!snap) {
      app.innerHTML = `<div class="sm-head"><div class="sm-title">系统监控</div></div>
        <div class="sm-muted">正在读取…</div>`;
      return;
    }

    const eff =
      snap.effectiveTempC != null && Number.isFinite(snap.effectiveTempC)
        ? Math.round(snap.effectiveTempC)
        : null;

    const cpu = snap.cpu || {};
    const mem = snap.memory || {};
    const disks = Array.isArray(snap.disks) ? snap.disks : [];
    const temps = Array.isArray(snap.temperatures) ? snap.temperatures : [];

    const diskHtml = disks.length
      ? disks
          .map((d) => {
            const label = escapeHtml(d.mount || d.name || "磁盘");
            const val = `${fmtBytes(d.usedBytes)} / ${fmtBytes(d.totalBytes)} · ${pct(d.usagePct)}`;
            return rowBar(label, val, d.usagePct);
          })
          .join("")
      : `<div class="sm-muted">无磁盘数据</div>`;

    const tempHtml = temps.length
      ? temps
          .map((t) => {
            const label = escapeHtml(`${t.label} (${t.kind})`);
            const val = `${Math.round(t.celsius)}°C`;
            const p = Math.max(0, Math.min(100, ((t.celsius - 30) / 70) * 100));
            return rowBar(label, val, p);
          })
          .join("")
      : `<div class="sm-muted">暂无温度传感器（Windows 常需驱动或管理员权限）</div>`;

    const cpuFreq =
      cpu.frequencyMhz > 0 ? ` · ${(cpu.frequencyMhz / 1000).toFixed(2)} GHz` : "";

    app.innerHTML = `
      <div class="sm-head">
        <div class="sm-title">系统监控</div>
        <div class="sm-eff">${
          eff != null
            ? `有效温度 <strong>${eff}°</strong>`
            : `<span class="sm-muted">有效温度 —</span>`
        }</div>
      </div>
      <div class="sm-body">
        <div class="sm-section">
          <div class="sm-section-title">CPU</div>
          ${rowBar(
            escapeHtml(cpu.brand || "CPU") +
              (cpu.coreCount ? ` · ${cpu.coreCount} 线程` : "") +
              cpuFreq,
            pct(cpu.usagePct),
            cpu.usagePct,
          )}
        </div>
        <div class="sm-section">
          <div class="sm-section-title">内存</div>
          ${rowBar(
            `${fmtBytes(mem.usedBytes)} / ${fmtBytes(mem.totalBytes)}`,
            pct(mem.usagePct),
            mem.usagePct,
          )}
        </div>
        <div class="sm-section">
          <div class="sm-section-title">磁盘</div>
          ${diskHtml}
        </div>
        <div class="sm-section">
          <div class="sm-section-title">温度 · max(CPU, GPU)=有效</div>
          <div class="sm-row">
            <span class="sm-row-label">CPU</span>
            <span class="sm-row-value">${
              snap.cpuTempC != null ? `${Math.round(snap.cpuTempC)}°C` : "—"
            }</span>
          </div>
          <div class="sm-row">
            <span class="sm-row-label">GPU</span>
            <span class="sm-row-value">${
              snap.gpuTempC != null ? `${Math.round(snap.gpuTempC)}°C` : "—"
            }</span>
          </div>
          ${tempHtml}
        </div>
      </div>
      <div class="sm-foot">约每秒刷新 · 基座采集</div>
    `;
  }

  async function tick() {
    try {
      state.snap = await hub().sysmon.snapshot();
      render();
    } catch (e) {
      console.error("[sysmon popup]", e);
      const app = document.getElementById("app");
      if (app) {
        app.innerHTML = `<div class="sm-title">系统监控</div>
          <div class="sm-muted">读取失败：${escapeHtml(e && e.message ? e.message : e)}</div>`;
      }
    }
  }

  function boot() {
    render();
    void tick();
    state.timer = window.setInterval(() => {
      void tick();
    }, POLL_MS);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }
})();
