import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { Adapter } from "./types";

export type TorrentSnapshot = {
  id: string;
  name: string;
  status: string;
  downloaded: number;
  uploaded: number;
  total: number;
  download_speed: number;
  upload_speed: number;
  eta_seconds: number | null;
  progress: number;
  peers: number;
  seeds: number | null;
  primary_adapter_name: string;
  interface_download_bytes: Record<string, number>;
  interface_upload_bytes: Record<string, number>;
  interface_download_speeds: Record<string, number>;
  interface_upload_speeds: Record<string, number>;
  binding_mode: string;
  error: string | null;
};
type Preview = {
  name: string;
  info_hash: string;
  total_size: number;
  binding_mode: string;
  files: { index: number; path: string; size: number; selected: boolean }[];
};
export const torrents = new Map<string, TorrentSnapshot>();
let currentAdapters: () => Adapter[] = () => [];
const escape = (s: string) =>
  s.replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!,
  );
const size = (n: number) =>
  n < 1024
    ? `${n} B`
    : n < 1048576
      ? `${(n / 1024).toFixed(1)} KiB`
      : n < 1073741824
        ? `${(n / 1048576).toFixed(1)} MiB`
        : `${(n / 1073741824).toFixed(2)} GiB`;

export function setupTorrents(getAdapters: () => Adapter[], getFolder: () => string) {
  currentAdapters = getAdapters;
  document
    .querySelector("nav")!
    .insertAdjacentHTML(
      "beforeend",
      '<button class="nav" data-view="torrents"><i>⇅</i>BitTorrent</button>',
    );
  document.querySelector("main")!.insertAdjacentHTML(
    "beforeend",
    `<section id="torrents-view" class="view">
    <div class="panel settings"><h2>Add a torrent</h2><p class="muted">Choose a .torrent file or paste a magnet URI. Review its files before downloading.</p>
    <label>Torrent source<input id="torrent-source" placeholder="magnet:?xt=urn:btih:…" spellcheck="false"></label>
    <div class="save-row"><button id="torrent-browse" class="secondary">Open .torrent file</button><button id="torrent-preview" class="primary">Review files</button></div>
    <p class="footnote">Torrent peer connections are spread across all selected adapters. Every peer socket is bound to its assigned Windows interface, and actual peer payload is reported per adapter.</p>
    <div id="torrent-metadata"></div><p id="torrent-error" class="form-error" role="alert"></p></div>
    <div class="section-head"><h2>Torrent queue</h2></div><div id="torrent-list" class="download-list"></div></section>`,
  );
  const source = document.querySelector<HTMLInputElement>("#torrent-source")!;
  const error = document.querySelector<HTMLElement>("#torrent-error")!;
  const metadata = document.querySelector<HTMLElement>("#torrent-metadata")!;
  let previewSource = "";
  let previewFolder = "";
  let previewAdapters: { id: string; name: string; local_ip: string }[] = [];
  source.addEventListener("input", () => {
    metadata.innerHTML = "";
  });
  document.querySelector("#torrent-browse")!.addEventListener("click", async () => {
    try {
      const path = await open({
        multiple: false,
        filters: [{ name: "BitTorrent", extensions: ["torrent"] }],
      });
      if (typeof path === "string") {
        source.value = path;
        metadata.innerHTML = "";
      }
    } catch (e) {
      error.textContent = String(e);
    }
  });
  document
    .querySelector<HTMLButtonElement>("#torrent-preview")!
    .addEventListener("click", async (e) => {
      const button = e.currentTarget as HTMLButtonElement;
      button.disabled = true;
      error.textContent = "Fetching metadata…";
      metadata.innerHTML = "";
      try {
        previewSource = source.value.trim();
        previewFolder = getFolder();
        previewAdapters = getAdapters().map((a) => ({
          id: a.id,
          name: a.name,
          local_ip: a.ipv4[0],
        }));
        const p = await invoke<Preview>("preview_torrent", {
          source: previewSource,
          destination: previewFolder,
          adapters: previewAdapters,
        });
        metadata.innerHTML = `<h3>${escape(p.name)}</h3><p class="muted">${size(p.total_size)} · ${p.files.length} files</p>
        <div class="torrent-files">${p.files.map((f) => `<label class="check"><input type="checkbox" data-torrent-file="${f.index}" ${f.selected ? "checked" : ""}><span>${escape(f.path)}</span><small>${size(f.size)}</small></label>`).join("")}</div>
        <div class="settings-grid"><label>Upload limit (KiB/s, 0 = unlimited)<input id="torrent-upload" type="number" value="256" min="0" max="1000000"></label><label>Maximum peers<input id="torrent-peers" type="number" value="80" min="1" max="500"></label></div>
        <label class="check"><input id="torrent-seed" type="checkbox"> Continue seeding after download completes</label>
        <p class="muted">${escape(p.binding_mode)}</p><p class="muted">Save to ${escape(previewFolder)}</p><button id="torrent-start" class="primary">Download selected files</button>`;
        document.querySelector("#torrent-start")!.addEventListener("click", async (event) => {
          const start = event.currentTarget as HTMLButtonElement;
          start.disabled = true;
          error.textContent = "";
          try {
            const files = Array.from(
              metadata.querySelectorAll<HTMLInputElement>("[data-torrent-file]:checked"),
            ).map((f) => Number(f.dataset.torrentFile));
            if (!files.length) throw new Error("Select at least one file.");
            // Refresh the selection at start time. Reviewing metadata is
            // intentionally allowed before the user chooses an interface.
            previewAdapters = getAdapters().map((a) => ({
              id: a.id,
              name: a.name,
              local_ip: a.ipv4[0],
            }));
            if (!previewAdapters.length)
              throw new Error("Select at least one connected adapter on the Network page.");
            await invoke("start_torrent", {
              request: {
                source: previewSource,
                destination_dir: previewFolder,
                selected_files: files,
                adapters: previewAdapters,
                upload_limit_kib: Number(
                  document.querySelector<HTMLInputElement>("#torrent-upload")!.value,
                ),
                peer_limit: Number(
                  document.querySelector<HTMLInputElement>("#torrent-peers")!.value,
                ),
                seed_after_download:
                  document.querySelector<HTMLInputElement>("#torrent-seed")!.checked,
              },
            });
            metadata.innerHTML = "";
            source.value = "";
          } catch (err) {
            error.textContent = String(err);
            start.disabled = false;
          }
        });
        error.textContent = "";
      } catch (err) {
        error.textContent = String(err);
      } finally {
        button.disabled = false;
      }
    });
  document.querySelector("#torrent-list")!.addEventListener("click", async (e) => {
    const button = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-torrent-action]");
    if (!button) return;
    try {
      await invoke(`${button.dataset.torrentAction}_torrent`, { id: button.dataset.id });
    } catch (err) {
      error.textContent = String(err);
    }
  });
  if ("__TAURI_INTERNALS__" in window) {
    void listen<TorrentSnapshot>("torrent-update", (e) => torrents.set(e.payload.id, e.payload));
    void invoke<TorrentSnapshot[]>("list_torrents")
      .then((items) => items.forEach((t) => torrents.set(t.id, t)))
      .catch((e) => {
        error.textContent = String(e);
      });
  }
  renderTorrents();
}

export function renderTorrents() {
  const list = document.querySelector("#torrent-list");
  if (!list) return;
  list.innerHTML = torrents.size
    ? Array.from(torrents.values())
        .map((t) => {
          const adapters = currentAdapters();
          const perInterface = Object.entries(t.interface_download_speeds)
            .map(([id, speed]) => {
              const adapter = adapters.find((a) => a.id === id);
              return `<span><b>${escape(adapter?.name || id)}</b> ↓ ${size(speed)}/s · ${size(t.interface_download_bytes[id] || 0)}</span>`;
            })
            .join("");
          return `<article class="download-card">
    <div class="download-top"><h3>${escape(t.name)}</h3><span>${escape(t.status)}</span></div>
    <div class="progress"><span style="width:${Math.max(0, Math.min(100, t.progress))}%"></span></div>
    <p>${t.progress.toFixed(1)}% · ${size(t.downloaded)} / ${size(t.total)} · ↓ ${size(t.download_speed)}/s · ↑ ${size(t.upload_speed)}/s</p>
    <p class="muted">${t.peers} peers · Seeds: ${t.seeds ?? "unavailable"} · ETA: ${t.eta_seconds == null ? "—" : `${Math.ceil(t.eta_seconds / 60)} min`} · Uploaded ${size(t.uploaded)}</p>
    <p class="muted">${escape(t.primary_adapter_name)} · ${escape(t.binding_mode)}</p>
    <div class="torrent-interface-stats">${perInterface}</div>
    ${t.error ? `<p class="form-error">${escape(t.error)}</p>` : ""}
    <button class="secondary" data-torrent-action="${t.status === "paused" ? "resume" : "pause"}" data-id="${escape(t.id)}">${t.status === "paused" ? "Resume / seed" : "Pause"}</button>
    </article>`;
        })
        .join("")
    : '<div class="empty compact"><h3>No torrents added</h3><p>Only download and share content you have permission to use.</p></div>';
}
