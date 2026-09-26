import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { Adapter, DownloadSnapshot, LogEvent, Settings } from "./types";
import "./styles.css";
import { setupTorrents, renderTorrents, torrents } from "./torrents";

const state = {
  adapters: [] as Adapter[],
  selected: new Set<string>(),
  downloads: new Map<string, DownloadSnapshot>(),
  logs: [] as LogEvent[],
  settings: null as Settings | null,
  speedHistory: new Map<string, number[]>(),
  activeView: "downloads",
};
const isTauri = "__TAURI_INTERNALS__" in window;
type InterfaceTraffic = {
  id: string;
  connected: boolean;
  received: number;
  sent: number;
  download_speed: number;
  upload_speed: number;
};
const interfaceTraffic = new Map<string, InterfaceTraffic>();

const app = document.querySelector<HTMLDivElement>("#app")!;
app.innerHTML = `
  <div class="shell">
    <aside class="sidebar">
      <div class="brand"><div class="brand-mark"><span></span><span></span><span></span></div><div><strong>NetBond</strong><small>Multi-network downloads</small></div></div>
      <nav>
        <button class="nav active" data-view="downloads"><i>⌁</i>Downloads <b id="active-count">0</b></button>
        <button class="nav" data-view="network"><i>◫</i>Network</button>
        <button class="nav" data-view="logs"><i>≡</i>Activity log</button>
        <button class="nav" data-view="settings"><i>⚙</i>Settings</button>
      </nav>
      <div class="sidebar-note"><span class="dot"></span><div><strong>Binding active</strong><small>Workers use selected source IPs</small></div></div>
    </aside>
    <main>
      <header><div><h1 id="page-title">Downloads</h1><p id="page-subtitle">Combine independent connections, one file at a time.</p></div><button id="refresh" class="icon-btn" title="Refresh adapters">↻</button></header>
      <section id="downloads-view" class="view active">
        <div class="notice"><span>i</span><p><strong>A quick reality check</strong> Ethernet and Wi-Fi connected to the same router usually share one WAN link, so their speeds may not add. A phone tether or separate ISP is more likely to increase total throughput.</p></div>
        <div class="new-download panel">
          <div class="panel-title"><div><h2>New download</h2><p>NetBond checks range support before creating workers.</p></div><span class="secure">◆ HTTPS preserved</span></div>
          <label>File URL<input id="url" type="url" placeholder="https://example.com/large-file.iso" spellcheck="false"></label>
          <div class="save-row"><label>Save folder<input id="destination" readonly></label><button id="browse" class="secondary">Browse</button></div>
          <div><span class="field-label">Use network adapters</span><div id="adapter-pills" class="adapter-pills"><div class="skeleton"></div></div></div>
          <div class="actions"><label class="check"><input id="overwrite" type="checkbox"> Replace an existing file with the same name</label><button id="start" class="primary"><span>↓</span> Start download</button></div>
          <p id="form-error" class="form-error"></p>
        </div>
        <div class="section-head"><div><h2>Download queue</h2><span id="queue-summary">No downloads yet</span></div><div class="filters"><button class="active">All</button><button>Active</button><button>Completed</button></div></div>
        <div id="download-list" class="download-list"><div class="empty"><div>↓</div><h3>Your queue is ready</h3><p>Add an HTTP or HTTPS URL to begin.</p></div></div>
      </section>
      <section id="network-view" class="view"><div class="speed-hero"><div><span>TOTAL DOWNLOAD SPEED</span><strong id="total-speed">0 B/s</strong><p>Combined NetBond traffic across selected adapters</p></div><div class="speed-mark">↓</div></div><div class="stats"><div><span>Selected adapters</span><strong id="selected-count">0</strong></div><div><span>Connected adapters</span><strong id="connected-count">0</strong></div></div><div class="section-head network-heading"><div><h2>Network interfaces</h2><span>Source-bound worker traffic</span></div></div><div id="network-list" class="network-grid"></div><p class="footnote">Traffic values show NetBond activity only, not the full capacity or usage of your internet connections.</p></section>
      <section id="logs-view" class="view"><div class="log-toolbar"><span>Connection and worker events</span><button id="clear-logs" class="secondary">Clear view</button></div><div id="log-list" class="logs"></div></section>
      <section id="settings-view" class="view"><form id="settings-form" class="settings panel"><h2>Download engine</h2><div class="settings-grid"><label>Connections per adapter<input name="connections_per_interface" type="number" min="1" max="8"></label><label>Chunk size (MiB)<input name="chunk_size_mib" type="number" min="1" max="128"></label><label>Retries per chunk<input name="retry_count" type="number" min="0" max="20"></label><label>Bandwidth limit (Mbps, 0 = unlimited)<input name="bandwidth_limit_mbps" type="number" min="0"></label></div><label>Default download folder<input name="download_directory"></label><label class="check"><input name="auto_use_new_adapters" type="checkbox"> Automatically select newly connected adapters</label><label class="check"><input name="start_minimized" type="checkbox"> Start minimized</label><div class="settings-actions"><span id="settings-result"></span><button class="primary" type="submit">Save settings</button></div></form></section>
    </main>
  </div>`;

const $ = <T extends HTMLElement>(selector: string) => document.querySelector<T>(selector)!;
const destinationInput = () => $("#destination") as HTMLInputElement;
$("#downloads-view").insertAdjacentHTML(
  "afterbegin",
  `<div class="speed-hero"><div><span>TOTAL DOWNLOAD SPEED</span><strong id="dashboard-speed">0 B/s</strong><p id="dashboard-total">0 B downloaded by NetBond</p></div><div class="speed-mark">↓</div></div>`,
);
const fmtBytes = (n: number) => {
  if (!Number.isFinite(n) || n <= 0) return "0 B";
  const u = ["B", "KiB", "MiB", "GiB", "TiB"];
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), u.length - 1);
  return `${(n / 1024 ** i).toFixed(i ? 1 : 0)} ${u[i]}`;
};
const fmtSpeed = (n: number) => `${fmtBytes(n)}/s`;
const fmtEta = (n: number | null) =>
  n == null
    ? "—"
    : n < 60
      ? `${Math.ceil(n)} sec`
      : n < 3600
        ? `${Math.ceil(n / 60)} min`
        : `${Math.floor(n / 3600)}h ${Math.ceil((n % 3600) / 60)}m`;

function adapterIcon(kind: Adapter["kind"]) {
  return kind === "Wi-Fi" ? "⌁" : kind === "USB tether" || kind === "Mobile" ? "▣" : "⇄";
}

function selectedAdapterRequests() {
  return state.adapters
    .filter((adapter) => state.selected.has(adapter.id) && adapter.connected && adapter.ipv4.length)
    .map((adapter) => ({ id: adapter.id, name: adapter.name, local_ip: adapter.ipv4[0] }));
}

function syncTorrentAdapters() {
  if (!isTauri) return;
  void invoke("update_torrent_adapters", { adapters: selectedAdapterRequests() }).catch((error) =>
    showError(`Could not update torrent adapters: ${error}`),
  );
}

function renderAdapters() {
  const usable = state.adapters.filter((a) => a.connected && a.ipv4.length);
  $("#adapter-pills").innerHTML = usable.length
    ? usable
        .map(
          (a) =>
            `<button class="adapter-pill ${state.selected.has(a.id) ? "selected" : ""}" data-adapter="${a.id}"><span>${adapterIcon(a.kind)}</span><div><strong>${esc(a.name)}</strong><small>${esc(a.ipv4[0])}</small></div><i>${state.selected.has(a.id) ? "✓" : "+"}</i></button>`,
        )
        .join("")
    : `<p class="muted">No connected IPv4 adapters found.</p>`;
  $("#network-list").innerHTML = state.adapters
    .map((a) => {
      const speed = aggregateInterfaceSpeed(a.id);
      const bytes = aggregateInterfaceBytes(a.id);
      const allBytes = aggregateAllInterfaceBytes();
      const share = allBytes ? (bytes / allBytes) * 100 : 0;
      const history = state.speedHistory.get(a.id) || [];
      const peak = Math.max(1, ...history);
      const points = history
        .map(
          (v, i) =>
            `${history.length < 2 ? 0 : (i / (history.length - 1)) * 100},${28 - (v / peak) * 24}`,
        )
        .join(" ");
      return `<article class="network-card ${a.connected ? "" : "offline"}"><div class="network-top"><div class="adapter-icon">${adapterIcon(a.kind)}</div><div><h3>${esc(a.name)}</h3><p>${esc(a.kind)} · ${esc(a.description || "Network adapter")}</p></div><label class="switch"><input type="checkbox" data-adapter="${a.id}" ${state.selected.has(a.id) ? "checked" : ""} ${!a.connected || !a.ipv4.length ? "disabled" : ""}><span></span></label></div><div class="network-status"><span class="status-dot"></span>${a.connected ? "Connected" : "Offline"}<strong>${fmtLink(a.link_speed_bps)}</strong></div><div class="live-rate"><div><span>DOWNLOAD</span><strong>${fmtSpeed(speed)}</strong></div><svg viewBox="0 0 100 30" preserveAspectRatio="none" aria-label="Recent speed graph"><polyline points="${points}"/></svg></div><dl><div><dt>Total through interface</dt><dd>${fmtBytes(bytes)}</dd></div><div><dt>Share of NetBond traffic</dt><dd class="teal">${share.toFixed(1)}%</dd></div><div><dt>IPv4 source</dt><dd>${esc(a.ipv4.join(", ") || "—")}</dd></div><div><dt>Gateway</dt><dd>${esc(a.gateways.join(", ") || "—")}</dd></div></dl><div class="binding-state">${a.connected && a.ipv4.length ? `Workers bind to ${esc(a.ipv4[0])}` : "New work paused until this interface reconnects"}</div></article>`;
    })
    .join("");
  document.querySelectorAll<HTMLElement>(".network-card").forEach((card, index) => {
    const adapter = state.adapters[index];
    const traffic = interfaceTraffic.get(adapter.id);
    const upload = Array.from(torrents.values()).reduce(
      (sum, t) => sum + (t.interface_upload_speeds[adapter.id] || 0),
      0,
    );
    card
      .querySelector("dl")!
      .insertAdjacentHTML(
        "beforeend",
        `<div><dt>Torrent upload</dt><dd>${fmtSpeed(upload)}</dd></div><div><dt>Windows receive · all apps</dt><dd>${traffic ? fmtSpeed(traffic.download_speed) : "Unavailable"}</dd></div><div><dt>Windows send · all apps</dt><dd>${traffic ? fmtSpeed(traffic.upload_speed) : "Unavailable"}</dd></div>`,
      );
  });
  $("#selected-count").textContent = String(state.selected.size);
  $("#connected-count").textContent = String(state.adapters.filter((a) => a.connected).length);
  document.querySelectorAll<HTMLElement>("[data-adapter]").forEach((el) =>
    el.addEventListener("click", (e) => {
      e.preventDefault();
      const id = el.dataset.adapter!;
      state.selected.has(id) ? state.selected.delete(id) : state.selected.add(id);
      renderAdapters();
      syncTorrentAdapters();
    }),
  );
}

function aggregateInterfaceSpeed(id: string) {
  let n = 0;
  state.downloads.forEach((d) => (n += d.interface_speeds[id] || 0));
  torrents.forEach((t) => (n += t.interface_download_speeds[id] || 0));
  return n;
}
function aggregateInterfaceBytes(id: string) {
  let n = 0;
  state.downloads.forEach((d) => (n += d.interface_bytes?.[id] || 0));
  torrents.forEach((t) => (n += t.interface_download_bytes[id] || 0));
  return n;
}
function aggregateAllInterfaceBytes() {
  return state.adapters.reduce((n, a) => n + aggregateInterfaceBytes(a.id), 0);
}
function recordSpeedHistory() {
  state.adapters.forEach((a) => {
    const values = state.speedHistory.get(a.id) || [];
    values.push(aggregateInterfaceSpeed(a.id));
    if (values.length > 32) values.shift();
    state.speedHistory.set(a.id, values);
  });
}

function renderDownloads() {
  const list = [...state.downloads.values()];
  $("#active-count").textContent = String(
    list.filter((d) => ["probing", "downloading"].includes(d.status)).length,
  );
  $("#queue-summary").textContent = list.length
    ? `${list.length} item${list.length === 1 ? "" : "s"}`
    : "No downloads yet";
  $("#total-speed").textContent = fmtSpeed(list.reduce((s, d) => s + d.bytes_per_second, 0));
  if (!list.length) return;
  $("#download-list").innerHTML = list
    .map((d) => {
      const pct = d.total ? Math.min(100, (d.downloaded / d.total) * 100) : 0;
      const canPause = d.status === "downloading" || d.status === "probing";
      const canResume = d.status === "paused" || d.status === "failed";
      return `<article class="download-card"><div class="file-icon">${d.status === "completed" ? "✓" : "↓"}</div><div class="download-body"><div class="download-title"><div><h3>${esc(d.filename || "Preparing download…")}</h3><p>${esc(d.url)}</p></div><span class="badge ${d.status}">${d.status}</span></div><div class="progress"><span style="width:${pct}%"></span></div><div class="metrics"><span><b>${pct.toFixed(1)}%</b> progress</span><span><b>${fmtBytes(d.downloaded)}</b> of ${d.total ? fmtBytes(d.total) : "unknown"}</span><span><b>${fmtSpeed(d.bytes_per_second)}</b> speed</span><span><b>${fmtEta(d.eta_seconds)}</b> remaining</span></div>${d.error ? `<p class="download-error">${esc(d.error)}</p>` : ""}<div class="download-footer"><span>${d.range_supported === false ? "Single connection · Server does not support ranges" : d.range_supported === true ? "Multi-interface range download" : "Checking server capabilities"}</span><div>${canPause ? `<button data-action="pause" data-id="${d.id}">Pause</button>` : ""}${canResume ? `<button data-action="resume" data-id="${d.id}">Resume</button>` : ""}${!["completed", "cancelled"].includes(d.status) ? `<button class="danger" data-action="cancel" data-id="${d.id}">Cancel</button>` : ""}</div></div></div></article>`;
    })
    .join("");
  document.querySelectorAll<HTMLButtonElement>("[data-action]").forEach((b) =>
    b.addEventListener("click", async () => {
      try {
        await invoke(`${b.dataset.action}_download`, { id: b.dataset.id });
      } catch (e) {
        showError(String(e));
      }
    }),
  );
}

function renderLogs() {
  $("#log-list").innerHTML = state.logs.length
    ? state.logs
        .slice()
        .reverse()
        .map(
          (l) =>
            `<div class="log ${l.level}"><time>${new Date(l.timestamp).toLocaleTimeString()}</time><span>${l.level}</span><p>${esc(l.message)}</p></div>`,
        )
        .join("")
    : `<div class="empty compact"><div>≡</div><h3>No activity yet</h3><p>Worker assignments, byte ranges, retries and HTTP responses appear here.</p></div>`;
}

const esc = (s: string) =>
  s.replace(
    /[&<>'"]/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" })[c]!,
  );
const fmtLink = (bps: number) =>
  bps ? `${(bps / 1_000_000).toFixed(bps < 100_000_000 ? 0 : 0)} Mbps link` : "Speed unavailable";
const showError = (text: string) => {
  $("#form-error").textContent = text;
};

async function refreshAdapters() {
  $("#refresh").classList.add("spin");
  try {
    if (!isTauri) {
      state.adapters = [];
      renderAdapters();
      showError(
        "Browser preview only. Open the Windows app to read real adapters and download files.",
      );
      return;
    }
    const known = new Set(state.adapters.map((a) => a.id));
    const fresh = await invoke<Adapter[]>("enumerate_adapters");
    if (state.settings?.auto_use_new_adapters)
      fresh
        .filter((a) => a.connected && a.ipv4.length && !known.has(a.id))
        .forEach((a) => state.selected.add(a.id));
    state.adapters = fresh;
    renderAdapters();
    syncTorrentAdapters();
  } catch (e) {
    showError(`Could not read network adapters: ${e}`);
  } finally {
    $("#refresh").classList.remove("spin");
  }
}

async function init() {
  try {
    if (!isTauri) {
      state.settings = {
        connections_per_interface: 3,
        chunk_size_mib: 8,
        retry_count: 4,
        download_directory: "C:\\Users\\You\\Downloads",
        bandwidth_limit_mbps: 0,
        auto_use_new_adapters: true,
        start_minimized: false,
      };
      destinationInput().value = state.settings.download_directory;
      document.querySelector<HTMLInputElement>("#torrent-destination")!.value =
        state.settings.download_directory;
      const form = $("#settings-form") as HTMLFormElement;
      Object.entries(state.settings).forEach(([k, v]) => {
        const input = form.elements.namedItem(k) as HTMLInputElement;
        if (input)
          input.type === "checkbox" ? (input.checked = Boolean(v)) : (input.value = String(v));
      });
      await refreshAdapters();
      return;
    }
    state.settings = await invoke<Settings>("get_settings");
    destinationInput().value = state.settings.download_directory;
    document.querySelector<HTMLInputElement>("#torrent-destination")!.value =
      state.settings.download_directory;
    const form = $("#settings-form") as HTMLFormElement;
    Object.entries(state.settings).forEach(([k, v]) => {
      const input = form.elements.namedItem(k) as HTMLInputElement;
      if (input)
        input.type === "checkbox" ? (input.checked = Boolean(v)) : (input.value = String(v));
    });
    const recovered = await invoke<DownloadSnapshot[]>("list_downloads");
    recovered.forEach((d) => state.downloads.set(d.id, d));
    renderDownloads();
  } catch (e) {
    showError(String(e));
  }
  await refreshAdapters();
}

$("#browse").addEventListener("click", async () => {
  const result = await open({
    directory: true,
    multiple: false,
    defaultPath: destinationInput().value,
  });
  if (typeof result === "string") destinationInput().value = result;
});
$("#start").addEventListener("click", async () => {
  showError("");
  const url = $("#url") as HTMLInputElement;
  if (!/^https?:\/\//i.test(url.value)) return showError("Enter a valid HTTP or HTTPS URL.");
  if (!state.selected.size) return showError("Select at least one connected adapter.");
  const adapters = state.adapters
    .filter((a) => state.selected.has(a.id) && a.connected && a.ipv4.length)
    .map((a) => ({ id: a.id, name: a.name, local_ip: a.ipv4[0] }));
  try {
    const id = await invoke<string>("start_download", {
      request: {
        url: url.value.trim(),
        destination_dir: destinationInput().value,
        adapters,
        overwrite: ($("#overwrite") as HTMLInputElement).checked,
      },
    });
    url.value = "";
    const snapshot = await invoke<DownloadSnapshot>("get_download", { id });
    state.downloads.set(id, snapshot);
    renderDownloads();
  } catch (e) {
    showError(String(e));
  }
});

setupTorrents(
  () => state.adapters.filter((a) => state.selected.has(a.id) && a.connected && a.ipv4.length),
  () => destinationInput().value,
);
document.querySelectorAll<HTMLButtonElement>(".nav").forEach((b) =>
  b.addEventListener("click", () => {
    state.activeView = b.dataset.view!;
    document.querySelectorAll(".nav,.view").forEach((x) => x.classList.remove("active"));
    b.classList.add("active");
    $(`#${b.dataset.view}-view`).classList.add("active");
    const names: Record<string, [string, string]> = {
      downloads: ["Downloads", "Combine independent connections, one file at a time."],
      network: ["Network dashboard", "Connected adapters and NetBond traffic."],
      logs: ["Activity log", "See exactly how every range and connection is handled."],
      settings: ["Settings", "Tune the download engine for your connections."],
    };
    names.torrents = ["BitTorrent", "Review files, control sharing, and follow real peer traffic."];
    $("#page-title").textContent = names[b.dataset.view!][0];
    $("#page-subtitle").textContent = names[b.dataset.view!][1];
  }),
);
$("#refresh").addEventListener("click", refreshAdapters);
$("#clear-logs").addEventListener("click", () => {
  state.logs = [];
  renderLogs();
});
$("#settings-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const form = e.currentTarget as HTMLFormElement;
  const data = new FormData(form);
  const settings: Settings = {
    connections_per_interface: Number(data.get("connections_per_interface")),
    chunk_size_mib: Number(data.get("chunk_size_mib")),
    retry_count: Number(data.get("retry_count")),
    download_directory: String(data.get("download_directory")),
    bandwidth_limit_mbps: Number(data.get("bandwidth_limit_mbps")),
    auto_use_new_adapters: data.has("auto_use_new_adapters"),
    start_minimized: data.has("start_minimized"),
  };
  try {
    await invoke("save_settings", { value: settings });
    state.settings = settings;
    $("#settings-result").textContent = "Saved";
    destinationInput().value = settings.download_directory;
  } catch (err) {
    $("#settings-result").textContent = String(err);
  }
});

if (isTauri) {
  listen<InterfaceTraffic[]>("interface-traffic", (e) =>
    e.payload.forEach((t) => {
      interfaceTraffic.set(t.id, t);
      const adapter = state.adapters.find((a) => a.id === t.id);
      if (adapter) adapter.connected = t.connected;
    }),
  );
  listen<DownloadSnapshot>("download-update", (e) => {
    state.downloads.set(e.payload.id, e.payload);
  });
  listen<LogEvent>("netbond-log", (e) => {
    state.logs.push(e.payload);
    if (state.logs.length > 1000) state.logs.shift();
    renderLogs();
  });
  listen("adapters-changed", refreshAdapters);
}
window.setInterval(() => {
  recordSpeedHistory();
  renderDownloads();
  if (state.activeView === "network") renderAdapters();
  if (state.activeView === "torrents") renderTorrents();
  const speed = state.adapters.reduce((sum, a) => sum + aggregateInterfaceSpeed(a.id), 0);
  $("#total-speed").textContent = fmtSpeed(speed);
  $("#dashboard-speed").textContent = fmtSpeed(speed);
  $("#dashboard-total").textContent =
    `${fmtBytes(aggregateAllInterfaceBytes())} downloaded by NetBond · payload bytes, including retries`;
}, 250);
init();
