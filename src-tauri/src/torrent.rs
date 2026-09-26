use crate::models::SelectedAdapter;
use librqbit::{
    limits::LimitsConfig, AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrent,
    ManagedTorrentState, PeerStatsFilter, PeerStatsFilterState, Session, SessionOptions,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentFile {
    pub index: usize,
    pub path: String,
    pub size: u64,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentPreview {
    pub name: String,
    pub info_hash: String,
    pub total_size: u64,
    pub files: Vec<TorrentFile>,
    pub primary_adapter: Option<SelectedAdapter>,
    pub binding_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentStartRequest {
    pub source: String,
    pub destination_dir: String,
    pub selected_files: Vec<usize>,
    pub adapters: Vec<SelectedAdapter>,
    pub upload_limit_kib: u32,
    pub peer_limit: usize,
    #[serde(default)]
    pub seed_after_download: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentSnapshot {
    pub id: String,
    pub name: String,
    pub status: String,
    pub downloaded: u64,
    pub uploaded: u64,
    pub total: u64,
    pub download_speed: u64,
    pub upload_speed: u64,
    pub eta_seconds: Option<f64>,
    pub progress: f64,
    pub peers: u32,
    pub seeds: Option<u32>,
    pub primary_adapter_id: String,
    pub primary_adapter_name: String,
    pub interface_download_bytes: HashMap<String, u64>,
    pub interface_upload_bytes: HashMap<String, u64>,
    pub interface_download_speeds: HashMap<String, u64>,
    pub interface_upload_speeds: HashMap<String, u64>,
    pub binding_mode: String,
    pub error: Option<String>,
}

struct TorrentRecord {
    handle: Arc<ManagedTorrent>,
    snapshot: TorrentSnapshot,
    seed_after_download: bool,
    completion_handled: bool,
}
struct Inner {
    session: Option<Arc<Session>>,
    bound: Vec<SelectedAdapter>,
    torrents: HashMap<String, TorrentRecord>,
}

#[derive(Clone)]
pub struct TorrentEngine {
    app: AppHandle,
    inner: Arc<Mutex<Inner>>,
}

impl TorrentEngine {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            inner: Arc::new(Mutex::new(Inner {
                session: None,
                bound: vec![],
                torrents: HashMap::new(),
            })),
        }
    }

    async fn session(
        &self,
        destination: &Path,
        adapters: &[SelectedAdapter],
    ) -> Result<Arc<Session>, String> {
        if adapters.is_empty() {
            return Err("Select at least one adapter".into());
        }
        let mut inner = self.inner.lock().await;
        if let Some(session) = &inner.session {
            let current = inner.bound.iter().map(|a| &a.local_ip).collect::<Vec<_>>();
            let requested = adapters.iter().map(|a| &a.local_ip).collect::<Vec<_>>();
            if current != requested {
                return Err("Torrent engine is already using a different adapter selection. Restart NetBond to change it.".into());
            }
            return Ok(session.clone());
        }
        let mut opts = SessionOptions::default();
        opts.bind_device_name = Some(
            adapters
                .iter()
                .map(|a| a.local_ip.as_str())
                .collect::<Vec<_>>()
                .join(";"),
        );
        opts.listen = None;
        opts.ipv4_only = true;
        opts.disable_local_service_discovery = true;
        opts.client_name_and_version = Some(format!("NetBond/0.1 rqbit/{}", librqbit::version()));
        let session = Session::new_with_opts(destination.to_path_buf(), opts)
            .await
            .map_err(|e| format!("Could not initialize source-bound torrent engine: {e:#}"))?;
        inner.session = Some(session.clone());
        inner.bound = adapters.to_vec();
        Ok(session)
    }

    pub async fn preview(
        &self,
        source: String,
        destination: String,
        adapters: Vec<SelectedAdapter>,
    ) -> Result<TorrentPreview, String> {
        validate_source(&source)?;
        // Metadata inspection must not create or lock the long-lived download
        // session. A local .torrent can be inspected offline, while a magnet
        // may use the normal Windows route when the user has not selected a
        // source adapter yet.
        let mut session_options = SessionOptions::default();
        if !adapters.is_empty() {
            session_options.bind_device_name = Some(
                adapters
                    .iter()
                    .map(|a| a.local_ip.as_str())
                    .collect::<Vec<_>>()
                    .join(";"),
            );
        }
        session_options.listen = None;
        session_options.ipv4_only = true;
        session_options.disable_local_service_discovery = true;
        session_options.client_name_and_version =
            Some(format!("NetBond/0.1 rqbit/{}", librqbit::version()));
        let session = Session::new_with_opts(PathBuf::from(&destination), session_options)
            .await
            .map_err(|e| format!("Could not initialize torrent metadata reader: {e:#}"))?;
        let mut opts = AddTorrentOptions::default();
        opts.list_only = true;
        opts.output_folder = Some(destination);
        let response = session
            .add_torrent(make_add(&source).await?, Some(opts))
            .await
            .map_err(|e| format!("Could not read torrent metadata: {e:#}"))?;
        let AddTorrentResponse::ListOnly(list) = response else {
            return Err("Torrent engine did not return preview metadata".into());
        };
        let files = list
            .info
            .iter_file_details()
            .enumerate()
            .map(|(index, f)| TorrentFile {
                index,
                path: f.filename.to_pathbuf().to_string_lossy().into_owned(),
                size: f.len,
                selected: !f.attrs().padding,
            })
            .collect::<Vec<_>>();
        let total_size = files.iter().filter(|f| f.selected).map(|f| f.size).sum();
        Ok(TorrentPreview {
            name: list
                .info
                .name()
                .map(|v| v.into_owned())
                .unwrap_or_else(|| "Unnamed torrent".into()),
            info_hash: list.info_hash.as_string(),
            total_size,
            files,
            primary_adapter: adapters.first().cloned(),
            binding_mode: if adapters.is_empty() {
                "Metadata reviewed using the normal Windows network route. Select adapters before downloading."
                    .into()
            } else {
                format!(
                    "Metadata reviewed through {} selected adapter(s).",
                    adapters.len()
                )
            },
        })
    }

    pub async fn start(&self, request: TorrentStartRequest) -> Result<String, String> {
        if request.selected_files.is_empty() {
            return Err("Select at least one torrent file".into());
        }
        validate_source(&request.source)?;
        let adapter = request
            .adapters
            .first()
            .cloned()
            .ok_or("Select at least one adapter")?;
        let adapters = request.adapters.clone();
        let seed_after_download = request.seed_after_download;
        let session = self
            .session(Path::new(&request.destination_dir), &adapters)
            .await?;
        let mut opts = AddTorrentOptions::default();
        opts.output_folder = Some(request.destination_dir);
        opts.only_files = Some(request.selected_files);
        opts.peer_limit = Some(request.peer_limit.clamp(1, 500));
        opts.overwrite = false;
        opts.ratelimits = LimitsConfig {
            upload_bps: NonZeroU32::new(request.upload_limit_kib.saturating_mul(1024)),
            download_bps: None,
        };
        let response = session
            .add_torrent(make_add(&request.source).await?, Some(opts))
            .await
            .map_err(|e| format!("Could not add torrent: {e:#}"))?;
        let handle = response.into_handle().ok_or("Torrent was not added")?;
        let id = Uuid::new_v4().to_string();
        let total = handle.stats().total_bytes;
        let snapshot = TorrentSnapshot {
            id: id.clone(),
            name: handle.name().unwrap_or_else(|| "Resolving metadata…".into()),
            status: "initializing".into(), downloaded: 0, uploaded: 0, total,
            download_speed: 0, upload_speed: 0, eta_seconds: None, progress: 0.0,
            peers: 0, seeds: None, primary_adapter_id: adapter.id.clone(),
            primary_adapter_name: adapters.iter().map(|a|a.name.as_str()).collect::<Vec<_>>().join(" + "),
            interface_download_bytes: adapters.iter().map(|a|(a.id.clone(),0)).collect(),
            interface_upload_bytes: adapters.iter().map(|a|(a.id.clone(),0)).collect(),
            interface_download_speeds: adapters.iter().map(|a|(a.id.clone(),0)).collect(),
            interface_upload_speeds: adapters.iter().map(|a|(a.id.clone(),0)).collect(),
            binding_mode: format!("Peer connections distributed across {} selected adapters with Windows interface routing.", adapters.len()),
            error: None,
        };
        self.inner.lock().await.torrents.insert(
            id.clone(),
            TorrentRecord {
                handle,
                snapshot,
                seed_after_download,
                completion_handled: false,
            },
        );
        let engine = self.clone();
        let poll_id = id.clone();
        tokio::spawn(async move {
            engine.poll(poll_id).await;
        });
        Ok(id)
    }

    pub async fn pause(&self, id: &str) -> Result<(), String> {
        let (session, handle) = {
            let inner = self.inner.lock().await;
            (
                inner.session.clone().ok_or("Torrent engine unavailable")?,
                inner
                    .torrents
                    .get(id)
                    .map(|r| r.handle.clone())
                    .ok_or("Torrent not found")?,
            )
        };
        session.pause(&handle).await.map_err(|e| e.to_string())
    }
    pub async fn resume(&self, id: &str) -> Result<(), String> {
        let (session, handle) = {
            let inner = self.inner.lock().await;
            (
                inner.session.clone().ok_or("Torrent engine unavailable")?,
                inner
                    .torrents
                    .get(id)
                    .map(|r| r.handle.clone())
                    .ok_or("Torrent not found")?,
            )
        };
        session.unpause(&handle).await.map_err(|e| e.to_string())
    }
    pub async fn list(&self) -> Vec<TorrentSnapshot> {
        self.inner
            .lock()
            .await
            .torrents
            .values()
            .map(|r| r.snapshot.clone())
            .collect()
    }

    async fn poll(&self, id: String) {
        let mut previous_interface_download = HashMap::<String, u64>::new();
        let mut previous_interface_upload = HashMap::<String, u64>::new();
        let mut last = std::time::Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let elapsed = last.elapsed().as_secs_f64().max(0.001);
            last = std::time::Instant::now();
            let (snapshot, auto_pause) = {
                let mut inner = self.inner.lock().await;
                let adapters = inner.bound.clone();
                let Some(record) = inner.torrents.get_mut(&id) else {
                    return;
                };
                let stats = record.handle.stats();
                let peer_stats = record.handle.with_state(|state| match state {
                    ManagedTorrentState::Live(live) => {
                        Some(live.per_peer_stats_snapshot(PeerStatsFilter {
                            state: PeerStatsFilterState::All,
                        }))
                    }
                    _ => None,
                });
                let mut interface_download = adapters
                    .iter()
                    .map(|a| (a.id.clone(), 0u64))
                    .collect::<HashMap<_, _>>();
                let mut interface_upload = adapters
                    .iter()
                    .map(|a| (a.id.clone(), 0u64))
                    .collect::<HashMap<_, _>>();
                if let Some(peer_stats) = peer_stats {
                    for (peer, peer_stats) in peer_stats.peers {
                        let Ok(peer) = peer.parse() else { continue };
                        let Some(source) = netbond_route::peer_source(peer) else {
                            continue;
                        };
                        let Some(adapter) =
                            adapters.iter().find(|a| a.local_ip == source.to_string())
                        else {
                            continue;
                        };
                        *interface_download.entry(adapter.id.clone()).or_default() +=
                            peer_stats.counters.fetched_bytes;
                        *interface_upload.entry(adapter.id.clone()).or_default() +=
                            peer_stats.counters.uploaded_bytes;
                    }
                }
                let interface_download_speeds = interface_download
                    .iter()
                    .map(|(aid, bytes)| {
                        (
                            aid.clone(),
                            (bytes
                                .saturating_sub(*previous_interface_download.get(aid).unwrap_or(&0))
                                as f64
                                / elapsed) as u64,
                        )
                    })
                    .collect::<HashMap<_, _>>();
                let interface_upload_speeds = interface_upload
                    .iter()
                    .map(|(aid, bytes)| {
                        (
                            aid.clone(),
                            (bytes.saturating_sub(*previous_interface_upload.get(aid).unwrap_or(&0))
                                as f64
                                / elapsed) as u64,
                        )
                    })
                    .collect::<HashMap<_, _>>();
                previous_interface_download = interface_download.clone();
                previous_interface_upload = interface_upload.clone();
                let down = interface_download_speeds.values().sum();
                let up = interface_upload_speeds.values().sum();
                let snapshot = &mut record.snapshot;
                snapshot.status = if stats.finished && stats.live.is_some() {
                    "seeding".into()
                } else {
                    stats.state.to_string()
                };
                snapshot.downloaded = stats.progress_bytes;
                snapshot.uploaded = stats.uploaded_bytes;
                snapshot.total = stats.total_bytes;
                snapshot.download_speed = down;
                snapshot.upload_speed = up;
                snapshot.eta_seconds = if stats.finished {
                    Some(0.0)
                } else if down > 0 {
                    Some(
                        stats.total_bytes.saturating_sub(stats.progress_bytes) as f64 / down as f64,
                    )
                } else {
                    None
                };
                snapshot.progress = if stats.total_bytes > 0 {
                    stats.progress_bytes as f64 / stats.total_bytes as f64 * 100.0
                } else {
                    0.0
                };
                snapshot.peers = stats
                    .live
                    .as_ref()
                    .map(|l| l.snapshot.peer_stats.live)
                    .unwrap_or(0);
                snapshot.error = stats.error;
                snapshot.interface_download_bytes = interface_download;
                snapshot.interface_upload_bytes = interface_upload;
                snapshot.interface_download_speeds = interface_download_speeds;
                snapshot.interface_upload_speeds = interface_upload_speeds;
                let auto_pause =
                    stats.finished && !record.seed_after_download && !record.completion_handled;
                if auto_pause {
                    record.completion_handled = true;
                }
                (snapshot.clone(), auto_pause)
            };
            let _ = self.app.emit("torrent-update", &snapshot);
            if auto_pause {
                let _ = self.pause(&id).await;
            }
            if snapshot.status == "error" {
                return;
            }
        }
    }
}

async fn make_add(source: &str) -> Result<AddTorrent<'static>, String> {
    if source.starts_with("magnet:") {
        Ok(AddTorrent::from_url(source.to_owned()))
    } else {
        let bytes = tokio::fs::read(source)
            .await
            .map_err(|e| format!("Cannot read .torrent file: {e}"))?;
        Ok(AddTorrent::from_bytes(bytes))
    }
}
fn validate_source(source: &str) -> Result<(), String> {
    if source.starts_with("magnet:?") {
        return Ok(());
    }
    let p = PathBuf::from(source);
    if p.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("torrent"))
        && p.is_file()
    {
        Ok(())
    } else {
        Err("Choose a .torrent file or enter a magnet URI".into())
    }
}
