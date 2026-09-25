use crate::models::{
    DownloadSnapshot, DownloadStatus, LogEvent, SelectedAdapter, Settings, StartRequest,
};
use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::IpAddr,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncSeekExt, AsyncWriteExt, SeekFrom},
    sync::{Mutex, RwLock},
    time::sleep,
};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum ChunkState {
    Pending,
    InFlight,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Chunk {
    start: u64,
    end: u64,
    state: ChunkState,
    attempts: usize,
    adapter_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedJob {
    snapshot: DownloadSnapshot,
    request: StartRequest,
    settings: Settings,
    chunks: Vec<Chunk>,
    part_path: PathBuf,
    final_path: PathBuf,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    last_modified: Option<String>,
}

struct JobData {
    snapshot: DownloadSnapshot,
    request: StartRequest,
    settings: Settings,
    chunks: Vec<Chunk>,
    part_path: PathBuf,
    final_path: PathBuf,
    sidecar_path: PathBuf,
    etag: Option<String>,
    last_modified: Option<String>,
}

struct Job {
    data: Mutex<JobData>,
    file: Mutex<Option<File>>,
    persistence: Mutex<()>,
}

#[derive(Clone)]
pub struct Engine {
    app: AppHandle,
    jobs: Arc<RwLock<HashMap<String, Arc<Job>>>>,
    settings: Arc<RwLock<Settings>>,
    adapters: Arc<RwLock<Option<Vec<crate::models::AdapterInfo>>>>,
}

struct Probe {
    total: Option<u64>,
    range_supported: bool,
    filename: String,
    etag: Option<String>,
    last_modified: Option<String>,
}

impl Engine {
    pub fn new(app: AppHandle, settings: Settings) -> Self {
        Self {
            app,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            settings: Arc::new(RwLock::new(settings)),
            adapters: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn current_settings(&self) -> Settings {
        self.settings.read().await.clone()
    }
    pub async fn set_settings(&self, value: Settings) {
        *self.settings.write().await = value;
    }
    pub async fn update_adapters(&self, value: Vec<crate::models::AdapterInfo>) {
        *self.adapters.write().await = Some(value);
    }
    pub async fn adapter_list(&self) -> Vec<crate::models::AdapterInfo> {
        self.adapters.read().await.clone().unwrap_or_default()
    }
    async fn adapter_online(&self, adapter: &SelectedAdapter) -> bool {
        self.adapters
            .read()
            .await
            .as_ref()
            .map(|list| {
                list.iter().any(|a| {
                    a.id == adapter.id
                        && a.connected
                        && a.ipv4.contains(&adapter.local_ip)
                        && crate::traffic::counters(a.interface_index)
                            .map(|c| c.0)
                            .unwrap_or(false)
                })
            })
            .unwrap_or(true)
    }

    pub async fn start(&self, request: StartRequest) -> Result<String, String> {
        validate_start_request(&request)?;
        let id = Uuid::new_v4().to_string();
        let settings = self.current_settings().await;
        let snapshot = DownloadSnapshot {
            id: id.clone(),
            url: request.url.clone(),
            filename: "Preparing download…".into(),
            destination: request.destination_dir.clone(),
            status: DownloadStatus::Waiting,
            downloaded: 0,
            total: None,
            bytes_per_second: 0.0,
            eta_seconds: None,
            range_supported: None,
            error: None,
            interface_speeds: HashMap::new(),
            interface_bytes: HashMap::new(),
            checksum_sha256: None,
        };
        let job = Arc::new(Job {
            data: Mutex::new(JobData {
                snapshot,
                request,
                settings,
                chunks: vec![],
                part_path: PathBuf::new(),
                final_path: PathBuf::new(),
                sidecar_path: PathBuf::new(),
                etag: None,
                last_modified: None,
            }),
            file: Mutex::new(None),
            persistence: Mutex::new(()),
        });
        self.jobs.write().await.insert(id.clone(), job.clone());
        self.emit_snapshot(&job).await;
        let engine = self.clone();
        tokio::spawn(async move {
            engine.run_job(job, false).await;
        });
        Ok(id)
    }

    pub async fn get(&self, id: &str) -> Result<DownloadSnapshot, String> {
        let job = self
            .jobs
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| "Download not found".to_string())?;
        let snapshot = job.data.lock().await.snapshot.clone();
        Ok(snapshot)
    }

    pub async fn list(&self) -> Vec<DownloadSnapshot> {
        let jobs: Vec<_> = self.jobs.read().await.values().cloned().collect();
        let mut out = Vec::with_capacity(jobs.len());
        for job in jobs {
            out.push(job.data.lock().await.snapshot.clone());
        }
        out
    }

    pub async fn pause(&self, id: &str) -> Result<(), String> {
        let job = self.job(id).await?;
        let mut d = job.data.lock().await;
        if matches!(
            d.snapshot.status,
            DownloadStatus::Downloading | DownloadStatus::Probing
        ) {
            d.snapshot.status = DownloadStatus::Paused;
            self.log(
                Some(id),
                "info",
                "Download paused; completed chunks were saved",
            );
        }
        drop(d);
        self.persist(&job).await;
        self.emit_snapshot(&job).await;
        Ok(())
    }

    pub async fn resume(&self, id: &str) -> Result<(), String> {
        let job = self.job(id).await?;
        let mut d = job.data.lock().await;
        if !matches!(
            d.snapshot.status,
            DownloadStatus::Paused | DownloadStatus::Failed
        ) {
            return Err("Only a paused or failed download can be resumed".into());
        }
        if d.part_path.as_os_str().is_empty() {
            return Err(
                "This download failed before resume metadata could be created; start it again"
                    .into(),
            );
        }
        d.snapshot.status = DownloadStatus::Downloading;
        d.snapshot.error = None;
        drop(d);
        let needs_restart = job.file.lock().await.is_none();
        self.log(Some(id), "info", "Download resumed");
        self.emit_snapshot(&job).await;
        if needs_restart {
            let engine = self.clone();
            let recovered_job = job.clone();
            tokio::spawn(async move {
                engine.run_job(recovered_job, true).await;
            });
        }
        Ok(())
    }

    pub async fn cancel(&self, id: &str) -> Result<(), String> {
        let job = self.job(id).await?;
        job.data.lock().await.snapshot.status = DownloadStatus::Cancelled;
        self.log(
            Some(id),
            "warn",
            "Download cancelled; partial data retained for safety",
        );
        self.persist(&job).await;
        self.emit_snapshot(&job).await;
        Ok(())
    }

    async fn job(&self, id: &str) -> Result<Arc<Job>, String> {
        self.jobs
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| "Download not found".to_string())
    }

    async fn run_job(&self, job: Arc<Job>, recovered: bool) {
        let id = job.data.lock().await.snapshot.id.clone();
        let result = self.run_job_inner(job.clone(), recovered).await;
        if let Err(error) = result {
            *job.file.lock().await = None;
            let mut d = job.data.lock().await;
            if d.snapshot.status != DownloadStatus::Cancelled {
                d.snapshot.status = DownloadStatus::Failed;
                d.snapshot.error = Some(error.clone());
            }
            drop(d);
            self.log(Some(&id), "error", &error);
            self.persist(&job).await;
            self.emit_snapshot(&job).await;
        }
    }

    async fn run_job_inner(&self, job: Arc<Job>, recovered: bool) -> Result<(), String> {
        let (request, settings, id) = {
            let mut d = job.data.lock().await;
            d.snapshot.status = DownloadStatus::Probing;
            (d.request.clone(), d.settings.clone(), d.snapshot.id.clone())
        };
        self.emit_snapshot(&job).await;
        let first = request.adapters.first().ok_or("No adapter selected")?;
        let probe_client = build_client(&first.local_ip)?;
        self.log(
            Some(&id),
            "info",
            &format!("Probe bound to {} ({})", first.name, first.local_ip),
        );
        let probe = probe_url(&probe_client, &request.url).await?;
        self.log(
            Some(&id),
            "info",
            &format!(
                "Range support: {}; size: {}",
                probe.range_supported,
                probe
                    .total
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ),
        );

        if !recovered {
            let destination_dir = PathBuf::from(&request.destination_dir);
            fs::create_dir_all(&destination_dir)
                .await
                .map_err(|e| format!("Cannot create destination folder: {e}"))?;
            let final_path = destination_dir.join(&probe.filename);
            if final_path.exists() && !request.overwrite {
                return Err(
                    "A file with this name already exists. Enable replacement to continue.".into(),
                );
            }
            let part_path =
                destination_dir.join(format!(".{}.netbond-{}.part", probe.filename, id));
            let sidecar_path =
                destination_dir.join(format!(".{}.netbond-{}.json", probe.filename, id));
            let chunks = if probe.range_supported {
                split_ranges(
                    probe
                        .total
                        .ok_or("Range response did not include a total size")?,
                    settings.chunk_size_mib * 1024 * 1024,
                )
                .into_iter()
                .map(|(start, end)| Chunk {
                    start,
                    end,
                    state: ChunkState::Pending,
                    attempts: 0,
                    adapter_id: None,
                })
                .collect()
            } else {
                vec![]
            };
            let mut d = job.data.lock().await;
            d.snapshot.filename = probe.filename.clone();
            d.snapshot.destination = final_path.to_string_lossy().into_owned();
            d.snapshot.total = probe.total;
            d.snapshot.range_supported = Some(probe.range_supported);
            d.final_path = final_path;
            d.part_path = part_path;
            d.sidecar_path = sidecar_path;
            d.chunks = chunks;
            d.etag = probe.etag.clone();
            d.last_modified = probe.last_modified.clone();
        } else {
            let mut d = job.data.lock().await;
            if d.snapshot.total != probe.total
                || d.etag
                    .as_ref()
                    .is_some_and(|tag| Some(tag) != probe.etag.as_ref())
                || d.last_modified
                    .as_ref()
                    .is_some_and(|date| Some(date) != probe.last_modified.as_ref())
            {
                return Err("The remote file changed. Start a new download; partial data has been preserved.".into());
            }
            // Without a validator, re-download all ranges rather than combine possibly different versions.
            if d.etag.is_none() && d.last_modified.is_none() {
                for chunk in &mut d.chunks {
                    chunk.state = ChunkState::Pending;
                }
                d.snapshot.downloaded = 0;
            }
            if d.snapshot.range_supported == Some(true) && !probe.range_supported {
                return Err("Server no longer supports resuming byte ranges".into());
            }
            d.snapshot.range_supported = Some(probe.range_supported);
            reset_interrupted_chunks(&mut d.chunks);
        }

        if !wait_until_runnable(&job).await {
            return Ok(());
        }

        let (part_path, total, range_supported) = {
            let d = job.data.lock().await;
            (
                d.part_path.clone(),
                d.snapshot.total,
                d.snapshot.range_supported == Some(true),
            )
        };
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&part_path)
            .await
            .map_err(|e| format!("Cannot open partial file: {e}"))?;
        if let Some(size) = total {
            file.set_len(size)
                .await
                .map_err(|e| format!("Cannot allocate output file: {e}"))?;
        }
        *job.file.lock().await = Some(file);
        {
            let mut d = job.data.lock().await;
            d.snapshot.status = DownloadStatus::Downloading;
            d.snapshot.error = None;
        }
        self.persist(&job).await;
        self.emit_snapshot(&job).await;
        let telemetry_engine = self.clone();
        let telemetry_job = job.clone();
        tokio::spawn(async move {
            telemetry_engine.telemetry_loop(telemetry_job).await;
        });

        if range_supported {
            self.run_range_workers(job.clone(), &probe, &request, &settings)
                .await?;
        } else {
            self.log(
                Some(&id),
                "warn",
                "Server ignored Range; using one source-bound connection",
            );
            self.run_single(
                job.clone(),
                &probe_client,
                &request.url,
                first,
                settings.bandwidth_limit_mbps,
            )
            .await?;
        }
        if job.data.lock().await.snapshot.status == DownloadStatus::Cancelled {
            return Ok(());
        }
        self.finalize(job.clone()).await
    }

    async fn run_range_workers(
        &self,
        job: Arc<Job>,
        probe: &Probe,
        request: &StartRequest,
        settings: &Settings,
    ) -> Result<(), String> {
        let mut tasks = Vec::new();
        let worker_count = settings.connections_per_interface * request.adapters.len();
        for adapter in &request.adapters {
            for number in 0..settings.connections_per_interface {
                let engine = self.clone();
                let job = job.clone();
                let adapter = adapter.clone();
                let probe = Probe {
                    total: probe.total,
                    range_supported: true,
                    filename: probe.filename.clone(),
                    etag: probe.etag.clone(),
                    last_modified: probe.last_modified.clone(),
                };
                let url = request.url.clone();
                let retries = settings.retry_count;
                let limit = settings.bandwidth_limit_mbps;
                tasks.push(tokio::spawn(async move {
                    engine
                        .worker_loop(
                            job,
                            adapter,
                            number,
                            &url,
                            &probe,
                            retries,
                            limit,
                            worker_count,
                        )
                        .await
                }));
            }
        }
        let mut failure = None;
        for task in tasks {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    failure = Some(e);
                }
                Err(e) => {
                    failure = Some(format!("Worker task stopped: {e}"));
                }
            }
        }
        let complete = job
            .data
            .lock()
            .await
            .chunks
            .iter()
            .all(|c| c.state == ChunkState::Complete);
        if !complete {
            return Err(failure.unwrap_or_else(|| "Not every byte range completed".into()));
        }
        Ok(())
    }

    async fn worker_loop(
        &self,
        job: Arc<Job>,
        adapter: SelectedAdapter,
        number: usize,
        url: &str,
        probe: &Probe,
        retries: usize,
        limit_mbps: f64,
        worker_count: usize,
    ) -> Result<(), String> {
        let client = build_client(&adapter.local_ip)?;
        let id = job.data.lock().await.snapshot.id.clone();
        self.log(
            Some(&id),
            "info",
            &format!(
                "Worker {}.{} bound to {}",
                adapter.name,
                number + 1,
                adapter.local_ip
            ),
        );
        loop {
            if !wait_until_runnable(&job).await {
                return Ok(());
            }
            if job
                .data
                .lock()
                .await
                .chunks
                .iter()
                .all(|c| c.state == ChunkState::Complete)
            {
                return Ok(());
            }
            if !self.adapter_online(&adapter).await {
                sleep(Duration::from_millis(500)).await;
                continue;
            }
            let index = {
                let mut d = job.data.lock().await;
                if let Some((i, c)) = d
                    .chunks
                    .iter_mut()
                    .enumerate()
                    .find(|(_, c)| c.state == ChunkState::Pending)
                {
                    c.state = ChunkState::InFlight;
                    c.adapter_id = Some(adapter.id.clone());
                    Some(i)
                } else {
                    None
                }
            };
            let Some(index) = index else {
                sleep(Duration::from_millis(100)).await;
                continue;
            };
            let (start, end) = {
                let d = job.data.lock().await;
                (d.chunks[index].start, d.chunks[index].end)
            };
            self.log(
                Some(&id),
                "info",
                &format!(
                    "{} worker {} assigned bytes {}-{}",
                    adapter.name,
                    number + 1,
                    start,
                    end
                ),
            );
            let mut last_error = String::new();
            let mut success = None;
            for attempt in 0..=retries {
                if !wait_until_runnable(&job).await {
                    return Ok(());
                }
                match self
                    .fetch_range_tracked(&client, url, start, end, probe, &job, &adapter.id)
                    .await
                {
                    Ok(bytes) => {
                        success = Some(bytes);
                        break;
                    }
                    Err(e) => {
                        last_error = e;
                        {
                            let mut d = job.data.lock().await;
                            d.chunks[index].attempts += 1;
                        }
                        self.log(
                            Some(&id),
                            "warn",
                            &format!(
                                "{} bytes {}-{} attempt {}/{}: {}",
                                adapter.name,
                                start,
                                end,
                                attempt + 1,
                                retries + 1,
                                last_error
                            ),
                        );
                        if attempt < retries {
                            sleep(Duration::from_millis(retry_delay_ms(attempt))).await;
                        }
                    }
                }
            }
            let Some(bytes) = success else {
                let mut d = job.data.lock().await;
                d.chunks[index].state = ChunkState::Pending;
                if d.chunks[index].attempts > (retries + 1) * d.request.adapters.len() {
                    d.snapshot.status = DownloadStatus::Failed;
                    return Err(format!(
                        "Range {start}-{end} failed across selected adapters: {last_error}"
                    ));
                }
                drop(d);
                sleep(Duration::from_secs(2)).await;
                continue;
            };
            let began = Instant::now();
            {
                let mut guard = job.file.lock().await;
                let file = guard.as_mut().ok_or("Partial file was closed")?;
                file.seek(SeekFrom::Start(start))
                    .await
                    .map_err(|e| format!("Seek failed: {e}"))?;
                file.write_all(&bytes)
                    .await
                    .map_err(|e| format!("Write failed: {e}"))?;
                file.flush()
                    .await
                    .map_err(|e| format!("Flush failed: {e}"))?;
            }
            throttle(limit_mbps, worker_count, bytes.len() as u64, began).await;
            {
                let mut d = job.data.lock().await;
                d.chunks[index].state = ChunkState::Complete;
                d.snapshot.downloaded = d
                    .chunks
                    .iter()
                    .filter(|c| c.state == ChunkState::Complete)
                    .map(|c| c.end - c.start + 1)
                    .sum();
            }
            self.persist(&job).await;
            self.emit_snapshot(&job).await;
        }
    }

    async fn run_single(
        &self,
        job: Arc<Job>,
        client: &Client,
        url: &str,
        adapter: &SelectedAdapter,
        limit_mbps: f64,
    ) -> Result<(), String> {
        let response = client
            .get(url)
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|e| format!("Download request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("Server returned HTTP {}", response.status()));
        }
        let mut stream = response.bytes_stream();
        let mut position = 0u64;
        while let Some(item) = stream.next().await {
            if !wait_until_runnable(&job).await {
                return Ok(());
            }
            let bytes = item.map_err(|e| format!("Connection interrupted: {e}"))?;
            {
                let mut guard = job.file.lock().await;
                let file = guard.as_mut().ok_or("Partial file was closed")?;
                file.seek(SeekFrom::Start(position))
                    .await
                    .map_err(|e| e.to_string())?;
                file.write_all(&bytes).await.map_err(|e| e.to_string())?;
            }
            position += bytes.len() as u64;
            let mut d = job.data.lock().await;
            *d.snapshot
                .interface_bytes
                .entry(adapter.id.clone())
                .or_default() += bytes.len() as u64;
            d.snapshot.downloaded = position;
            drop(d);
            throttle(limit_mbps, 1, bytes.len() as u64, Instant::now()).await;
            self.emit_snapshot(&job).await;
        }
        if job
            .data
            .lock()
            .await
            .snapshot
            .total
            .is_some_and(|total| total != position)
        {
            return Err("Response length differs from the expected file size".into());
        }
        let mut file_guard = job.file.lock().await;
        let file = file_guard.as_mut().ok_or("Partial file was closed")?;
        file.set_len(position).await.map_err(|e| e.to_string())?;
        file.flush().await.map_err(|e| e.to_string())?;
        drop(file_guard);
        self.persist(&job).await;
        Ok(())
    }

    async fn fetch_range_tracked(
        &self,
        client: &Client,
        url: &str,
        start: u64,
        end: u64,
        probe: &Probe,
        job: &Arc<Job>,
        adapter_id: &str,
    ) -> Result<Vec<u8>, String> {
        let mut req = client
            .get(url)
            .header(header::RANGE, format!("bytes={start}-{end}"))
            .header(header::ACCEPT_ENCODING, "identity");
        if let Some(tag) = &probe.etag {
            req = req.header(header::IF_RANGE, tag);
        } else if let Some(date) = &probe.last_modified {
            req = req.header(header::IF_RANGE, date);
        }
        let response = req.send().await.map_err(|e| e.to_string())?;
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(format!("Expected HTTP 206, received {}", response.status()));
        }
        let content_range = response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !content_range.starts_with(&format!("bytes {start}-{end}/")) {
            return Err(format!("Mismatched Content-Range: {content_range}"));
        }
        let expected = end - start + 1;
        let mut output = Vec::with_capacity(expected as usize);
        let mut stream = response.bytes_stream();
        while let Some(item) = stream.next().await {
            if !wait_until_runnable(job).await {
                return Err("Download stopped".into());
            }
            let bytes = item.map_err(|e| e.to_string())?;
            {
                let mut d = job.data.lock().await;
                *d.snapshot
                    .interface_bytes
                    .entry(adapter_id.to_owned())
                    .or_default() += bytes.len() as u64;
            }
            if output.len() as u64 + bytes.len() as u64 > expected {
                return Err("Server sent more bytes than the requested range".into());
            }
            output.extend_from_slice(&bytes);
        }
        if output.len() as u64 != expected {
            return Err(format!(
                "Expected {expected} bytes, received {}",
                output.len()
            ));
        }
        Ok(output)
    }

    async fn telemetry_loop(&self, job: Arc<Job>) {
        let mut previous = job.data.lock().await.snapshot.interface_bytes.clone();
        let mut last = Instant::now();
        loop {
            sleep(Duration::from_millis(250)).await;
            let now = Instant::now();
            let elapsed = now.duration_since(last).as_secs_f64().max(0.001);
            last = now;
            let mut d = job.data.lock().await;
            let status = d.snapshot.status.clone();
            let current = d.snapshot.interface_bytes.clone();
            let terminal = matches!(
                status,
                DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Cancelled
            );
            let mut speeds = HashMap::new();
            for (id, bytes) in &current {
                speeds.insert(
                    id.clone(),
                    if terminal || status == DownloadStatus::Paused {
                        0.0
                    } else {
                        bytes.saturating_sub(*previous.get(id).unwrap_or(&0)) as f64 / elapsed
                    },
                );
            }
            previous = current;
            d.snapshot.interface_speeds = speeds;
            d.snapshot.bytes_per_second = d.snapshot.interface_speeds.values().sum();
            let rate = d.snapshot.bytes_per_second;
            d.snapshot.eta_seconds = d.snapshot.total.and_then(|t| {
                if rate > 0.0 {
                    Some(t.saturating_sub(d.snapshot.downloaded) as f64 / rate)
                } else {
                    None
                }
            });
            drop(d);
            self.emit_snapshot(&job).await;
            if terminal {
                break;
            }
        }
    }

    async fn finalize(&self, job: Arc<Job>) -> Result<(), String> {
        if let Some(file) = job.file.lock().await.as_mut() {
            file.sync_all().await.map_err(|e| e.to_string())?;
        }
        *job.file.lock().await = None;
        let (part, final_path, overwrite, id) = {
            let d = job.data.lock().await;
            (
                d.part_path.clone(),
                d.final_path.clone(),
                d.request.overwrite,
                d.snapshot.id.clone(),
            )
        };
        let checksum_path = part.clone();
        let checksum = tokio::task::spawn_blocking(move || sha256_file(&checksum_path))
            .await
            .map_err(|e| e.to_string())??;
        let backup = final_path.with_extension(format!("{id}.netbond-backup"));
        if final_path.exists() {
            if !overwrite {
                return Err("Destination appeared during download; the completed partial file was preserved".into());
            }
            fs::rename(&final_path, &backup)
                .await
                .map_err(|e| format!("Could not preserve existing file before replacement: {e}"))?;
        }
        if let Err(e) = fs::rename(&part, &final_path).await {
            if backup.exists() {
                let _ = fs::rename(&backup, &final_path).await;
            }
            return Err(format!("Atomic finalization failed: {e}"));
        }
        if backup.exists() {
            let _ = fs::remove_file(&backup).await;
        }
        let sidecar = {
            let mut d = job.data.lock().await;
            d.snapshot.status = DownloadStatus::Completed;
            d.snapshot.bytes_per_second = 0.0;
            d.snapshot.eta_seconds = Some(0.0);
            d.snapshot.checksum_sha256 = Some(checksum.clone());
            d.sidecar_path.clone()
        };
        if sidecar.exists() {
            let _ = fs::remove_file(sidecar).await;
        }
        self.log(Some(&id), "info", &format!("Complete; SHA-256 {checksum}"));
        self.emit_snapshot(&job).await;
        Ok(())
    }

    async fn persist(&self, job: &Arc<Job>) {
        let _guard = job.persistence.lock().await;
        let (path, persisted) = {
            let d = job.data.lock().await;
            if d.sidecar_path.as_os_str().is_empty()
                || d.snapshot.status == DownloadStatus::Completed
            {
                return;
            }
            (
                d.sidecar_path.clone(),
                PersistedJob {
                    snapshot: d.snapshot.clone(),
                    request: d.request.clone(),
                    settings: d.settings.clone(),
                    chunks: d.chunks.clone(),
                    part_path: d.part_path.clone(),
                    final_path: d.final_path.clone(),
                    etag: d.etag.clone(),
                    last_modified: d.last_modified.clone(),
                },
            )
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&persisted) {
            if let Err(e) = crate::settings::atomic_write(&path, &bytes) {
                self.log(
                    Some(&persisted.snapshot.id),
                    "error",
                    &format!("Could not save recovery metadata: {e}"),
                );
            }
        }
    }

    async fn emit_snapshot(&self, job: &Arc<Job>) {
        let snapshot = job.data.lock().await.snapshot.clone();
        let _ = self.app.emit("download-update", snapshot);
    }
    fn log(&self, id: Option<&str>, level: &str, message: &str) {
        let _ = self.app.emit(
            "netbond-log",
            LogEvent {
                timestamp: chrono::Utc::now().to_rfc3339(),
                level: level.into(),
                download_id: id.map(str::to_owned),
                message: message.into(),
            },
        );
    }

    pub async fn recover_from(&self, directory: &Path) {
        let Ok(mut entries) = fs::read_dir(directory).await else {
            return;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if !path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with('.') && n.contains(".netbond-") && n.ends_with(".json"))
                .unwrap_or(false)
            {
                continue;
            }
            let Ok(bytes) = fs::read(&path).await else {
                continue;
            };
            let Ok(mut p) = serde_json::from_slice::<PersistedJob>(&bytes) else {
                continue;
            };
            if matches!(
                p.snapshot.status,
                DownloadStatus::Completed | DownloadStatus::Cancelled
            ) || !p.part_path.is_file()
            {
                continue;
            }
            p.snapshot.bytes_per_second = 0.0;
            p.snapshot.interface_speeds.clear();
            p.snapshot.status = DownloadStatus::Paused;
            p.snapshot.error = Some("Recovered after restart. Select Resume to continue.".into());
            let id = p.snapshot.id.clone();
            let job = Arc::new(Job {
                data: Mutex::new(JobData {
                    snapshot: p.snapshot,
                    request: p.request,
                    settings: p.settings,
                    chunks: p.chunks,
                    part_path: p.part_path,
                    final_path: p.final_path,
                    sidecar_path: path,
                    etag: p.etag,
                    last_modified: p.last_modified,
                }),
                file: Mutex::new(None),
                persistence: Mutex::new(()),
            });
            self.jobs.write().await.insert(id.clone(), job);
            self.log(Some(&id), "info", "Recovered resumable download metadata");
        }
    }
}

fn validate_start_request(request: &StartRequest) -> Result<(), String> {
    let url = Url::parse(&request.url).map_err(|_| "Enter a valid URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only HTTP and HTTPS URLs are supported".into());
    }
    if request.adapters.is_empty() {
        return Err("Select at least one adapter".into());
    }
    for a in &request.adapters {
        IpAddr::from_str(&a.local_ip)
            .map_err(|_| format!("{} has no valid local address", a.name))?;
    }
    if request.destination_dir.trim().is_empty() {
        return Err("Choose a destination folder".into());
    }
    Ok(())
}

fn build_client(local_ip: &str) -> Result<Client, String> {
    let ip = IpAddr::from_str(local_ip).map_err(|e| format!("Invalid source address: {e}"))?;
    Client::builder()
        .local_address(ip)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("Too many redirects")
            } else if attempt.url().scheme() == "http"
                && attempt.previous().iter().any(|u| u.scheme() == "https")
            {
                attempt.error("HTTPS downgrade refused")
            } else {
                attempt.follow()
            }
        }))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(120))
        .pool_max_idle_per_host(8)
        .gzip(false)
        .brotli(false)
        .deflate(false)
        .user_agent("NetBond/0.1")
        .build()
        .map_err(|e| format!("Cannot create source-bound client: {e}"))
}

async fn probe_url(client: &Client, url: &str) -> Result<Probe, String> {
    let parsed = Url::parse(url).map_err(|e| e.to_string())?;
    let head = client
        .head(url)
        .header(header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .ok();
    let head_total = head.as_ref().and_then(|r| r.content_length());
    let etag = head
        .as_ref()
        .and_then(|r| r.headers().get(header::ETAG))
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let last_modified = head
        .as_ref()
        .and_then(|r| r.headers().get(header::LAST_MODIFIED))
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let disposition = head
        .as_ref()
        .and_then(|r| r.headers().get(header::CONTENT_DISPOSITION))
        .and_then(|v| v.to_str().ok());
    let filename = sanitize_filename(
        disposition
            .and_then(content_disposition_filename)
            .or_else(|| {
                parsed
                    .path_segments()
                    .and_then(|mut s| s.next_back())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("download.bin"),
    );
    let range = client
        .get(url)
        .header(header::RANGE, "bytes=0-0")
        .header(header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|e| format!("Could not reach the URL through the selected adapter: {e}"))?;
    let range_supported = range.status() == StatusCode::PARTIAL_CONTENT;
    let range_total = range
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.parse().ok());
    if !range_supported && !range.status().is_success() {
        return Err(format!("Server returned HTTP {}", range.status()));
    }
    Ok(Probe {
        total: range_total.or(head_total),
        range_supported,
        filename,
        etag,
        last_modified,
    })
}

#[cfg(test)]
async fn fetch_range(
    client: &Client,
    url: &str,
    start: u64,
    end: u64,
    probe: &Probe,
) -> Result<Vec<u8>, String> {
    let mut req = client
        .get(url)
        .header(header::RANGE, format!("bytes={start}-{end}"))
        .header(header::ACCEPT_ENCODING, "identity");
    if let Some(tag) = &probe.etag {
        req = req.header(header::IF_RANGE, tag);
    } else if let Some(date) = &probe.last_modified {
        req = req.header(header::IF_RANGE, date);
    }
    let response = req.send().await.map_err(|e| e.to_string())?;
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(format!("Expected HTTP 206, received {}", response.status()));
    }
    let expected = end - start + 1;
    let content_range = response
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_range.starts_with(&format!("bytes {start}-{end}/")) {
        return Err(format!("Mismatched Content-Range: {content_range}"));
    }
    let bytes = response.bytes().await.map_err(|e| e.to_string())?.to_vec();
    if bytes.len() as u64 != expected {
        return Err(format!(
            "Expected {expected} bytes, received {}",
            bytes.len()
        ));
    }
    Ok(bytes)
}

pub fn split_ranges(total: u64, chunk_size: u64) -> Vec<(u64, u64)> {
    if total == 0 || chunk_size == 0 {
        return vec![];
    }
    let mut ranges = vec![];
    let mut start = 0;
    while start < total {
        let end = (start + chunk_size - 1).min(total - 1);
        ranges.push((start, end));
        start = end + 1;
    }
    ranges
}

fn sanitize_filename(input: &str) -> String {
    let mut s: String = input
        .trim()
        .trim_matches('"')
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    s = s.trim_matches([' ', '.']).to_string();
    let upper = s.to_ascii_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if s.is_empty() {
        "download.bin".into()
    } else if reserved
        .iter()
        .any(|r| upper == *r || upper.starts_with(&format!("{r}.")))
    {
        format!("_{s}")
    } else {
        s.chars().take(180).collect()
    }
}

fn content_disposition_filename(value: &str) -> Option<&str> {
    value
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("filename=").map(|v| v.trim_matches('"')))
}
fn retry_delay_ms(attempt: usize) -> u64 {
    300 * 2u64.pow(attempt.min(5) as u32)
}
fn reset_interrupted_chunks(chunks: &mut [Chunk]) {
    for chunk in chunks {
        if chunk.state == ChunkState::InFlight {
            chunk.state = ChunkState::Pending;
        }
    }
}
async fn wait_until_runnable(job: &Arc<Job>) -> bool {
    loop {
        let status = job.data.lock().await.snapshot.status.clone();
        match status {
            DownloadStatus::Paused => sleep(Duration::from_millis(250)).await,
            DownloadStatus::Cancelled | DownloadStatus::Failed => return false,
            _ => return true,
        }
    }
}
async fn throttle(limit_mbps: f64, workers: usize, bytes: u64, started: Instant) {
    if limit_mbps <= 0.0 {
        return;
    }
    let target = bytes as f64 * 8.0 / (limit_mbps * 1_000_000.0 / workers.max(1) as f64);
    let elapsed = started.elapsed().as_secs_f64();
    if target > elapsed {
        sleep(Duration::from_secs_f64(target - elapsed)).await;
    }
}
fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        extract::State,
        http::{HeaderMap, Response},
        routing::get,
        Router,
    };
    use std::sync::Arc as StdArc;
    #[test]
    fn ranges_cover_exactly() {
        assert_eq!(split_ranges(10, 4), vec![(0, 3), (4, 7), (8, 9)]);
    }
    #[test]
    fn empty_ranges_are_safe() {
        assert!(split_ranges(0, 4).is_empty());
        assert!(split_ranges(20, 0).is_empty());
    }
    #[test]
    fn filename_is_windows_safe() {
        assert_eq!(sanitize_filename("a<b>:c?.iso"), "a_b__c_.iso");
        assert_eq!(sanitize_filename("CON"), "_CON");
    }
    #[test]
    fn validates_url_scheme() {
        let mut r = StartRequest {
            url: "file:///x".into(),
            destination_dir: "x".into(),
            adapters: vec![SelectedAdapter {
                id: "1".into(),
                name: "a".into(),
                local_ip: "127.0.0.1".into(),
            }],
            overwrite: false,
        };
        assert!(validate_start_request(&r).is_err());
        r.url = "https://example.com/a".into();
        assert!(validate_start_request(&r).is_ok());
    }
    #[test]
    fn checksum_is_stable() {
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(), b"netbond").unwrap();
        assert_eq!(
            sha256_file(f.path()).unwrap(),
            "3d8ea1dee7c5254cd003264a653db96f04192e8c60a6f5adcec30f5525cfe174"
        );
    }
    #[test]
    fn retry_backoff_is_bounded() {
        assert_eq!(retry_delay_ms(0), 300);
        assert_eq!(retry_delay_ms(3), 2400);
        assert_eq!(retry_delay_ms(99), 9600);
    }
    #[test]
    fn interrupted_ranges_become_resumable() {
        let mut chunks = vec![
            Chunk {
                start: 0,
                end: 3,
                state: ChunkState::Complete,
                attempts: 0,
                adapter_id: None,
            },
            Chunk {
                start: 4,
                end: 7,
                state: ChunkState::InFlight,
                attempts: 1,
                adapter_id: Some("gone".into()),
            },
        ];
        reset_interrupted_chunks(&mut chunks);
        assert_eq!(chunks[0].state, ChunkState::Complete);
        assert_eq!(chunks[1].state, ChunkState::Pending);
    }

    async fn range_handler(
        State(data): State<StdArc<Vec<u8>>>,
        headers: HeaderMap,
    ) -> Response<Body> {
        if let Some(range) = headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("bytes="))
        {
            let mut parts = range.split('-');
            let start: usize = parts.next().unwrap().parse().unwrap();
            let end: usize = parts.next().unwrap().parse().unwrap();
            return Response::builder()
                .status(206)
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {start}-{end}/{}", data.len()),
                )
                .header(header::CONTENT_LENGTH, end - start + 1)
                .body(Body::from(data[start..=end].to_vec()))
                .unwrap();
        }
        Response::builder()
            .status(200)
            .header(header::CONTENT_LENGTH, data.len())
            .body(Body::from(data.as_ref().clone()))
            .unwrap()
    }

    #[tokio::test]
    async fn local_range_server_splits_and_merges_without_corruption() {
        let source = StdArc::new((0..=255u8).cycle().take(4097).collect::<Vec<_>>());
        let app = Router::new()
            .route("/file.bin", get(range_handler).head(range_handler))
            .with_state(source.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let url = format!("http://{address}/file.bin");
        let client = build_client("127.0.0.1").unwrap();
        let probe = probe_url(&client, &url).await.unwrap();
        assert!(probe.range_supported);
        assert_eq!(probe.total, Some(source.len() as u64));
        let mut merged = Vec::new();
        for (start, end) in split_ranges(source.len() as u64, 511) {
            merged.extend(
                fetch_range(&client, &url, start, end, &probe)
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(merged, *source);
        server.abort();
    }
}
