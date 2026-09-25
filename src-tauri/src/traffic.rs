use serde::Serialize;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize)]
struct InterfaceTraffic {
    id: String,
    connected: bool,
    received: u64,
    sent: u64,
    download_speed: f64,
    upload_speed: f64,
}

pub fn counters(index: u32) -> Option<(bool, u64, u64)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::NetworkManagement::IpHelper::{GetIfEntry2, MIB_IF_ROW2};
        let mut row = MIB_IF_ROW2 {
            InterfaceIndex: index,
            ..Default::default()
        };
        // SAFETY: the API receives a correctly sized, initialized row.
        if unsafe { GetIfEntry2(&mut row) } != 0 {
            return None;
        }
        Some((row.OperStatus == 1, row.InOctets, row.OutOctets))
    }
    #[cfg(not(windows))]
    {
        let _ = index;
        None
    }
}

pub async fn monitor(app: AppHandle, engine: crate::engine::Engine) {
    let mut previous = HashMap::<String, (u64, u64, Instant)>::new();
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let mut output = Vec::new();
        for adapter in engine.adapter_list().await {
            let now = Instant::now();
            if let Some((connected, received, sent)) = counters(adapter.interface_index) {
                let (down, up) = previous
                    .get(&adapter.id)
                    .map(|&(r, s, time)| {
                        let elapsed = now.duration_since(time).as_secs_f64().max(0.001);
                        (
                            received.saturating_sub(r) as f64 / elapsed,
                            sent.saturating_sub(s) as f64 / elapsed,
                        )
                    })
                    .unwrap_or((0.0, 0.0));
                previous.insert(adapter.id.clone(), (received, sent, now));
                output.push(InterfaceTraffic {
                    id: adapter.id,
                    connected,
                    received,
                    sent,
                    download_speed: down,
                    upload_speed: up,
                });
            }
        }
        let _ = app.emit("interface-traffic", output);
    }
}
