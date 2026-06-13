use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};

use cdus_common::{ClipboardEvent, IpcMessage, TransportType};
use interprocess::local_socket::LocalSocketStream;
use std::io::{Read, Write};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct AppState {
    last_synced: Mutex<Option<SystemTime>>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn get_socket_path() -> String {
    std::env::var("CDUS_AGENT_SOCKET").unwrap_or_else(|_| "/tmp/cdus-agent.sock".to_string())
}

fn send_ipc_message(msg: IpcMessage) -> Result<IpcMessage, String> {
    let socket_name = get_socket_path();
    let mut stream = LocalSocketStream::connect(socket_name)
        .map_err(|e| format!("Failed to connect to agent: {}", e))?;

    let bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;

    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).map_err(|e| e.to_string())?;

    if buffer.is_empty() {
        return Err("Agent closed connection without response".to_string());
    }

    serde_json::from_slice(&buffer).map_err(|e| format!("Failed to parse response: {}", e))
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

fn check_agent_online() -> bool {
    let msg = IpcMessage::Ping;
    match send_ipc_message(msg) {
        Ok(IpcMessage::Pong) => true,
        _ => false,
    }
}

#[tauri::command]
fn ping_agent() -> Result<String, String> {
    if check_agent_online() {
        Ok("Pong".to_string())
    } else {
        Err("Failed to connect to agent".to_string())
    }
}

#[tauri::command]
fn read_system_clipboard() -> Result<String, String> {
    use arboard::Clipboard;
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.get_text().map_err(|e| e.to_string())
}

#[tauri::command]
fn broadcast_clipboard(content: String) -> Result<String, String> {
    let msg = IpcMessage::ClipboardChanged {
        content,
        timestamp: now_ms(),
    };

    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        response => Ok(format!("{:?}", response)),
    }
}

#[tauri::command]
fn set_clipboard(content: String) -> Result<String, String> {
    let msg = IpcMessage::SetClipboard {
        content,
        timestamp: now_ms(),
        source: "Local".to_string(),
    };

    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        response => Ok(format!("{:?}", response)),
    }
}

#[tauri::command]
fn get_clipboard_history(
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<ClipboardEvent>, String> {
    let msg = IpcMessage::GetHistory { limit };

    match send_ipc_message(msg)? {
        IpcMessage::HistoryResponse(history) => {
            if !history.is_empty() {
                let mut ls = state.last_synced.lock().unwrap();
                *ls = Some(SystemTime::now());
            }
            Ok(history)
        }
        IpcMessage::Log(err) => Err(err),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
fn delete_clipboard_item(id: i64) -> Result<String, String> {
    let msg = IpcMessage::DeleteHistoryItem { id };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
fn clear_clipboard_history() -> Result<String, String> {
    let msg = IpcMessage::ClearHistory;
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
fn toggle_local_only(id: i64, local_only: bool) -> Result<String, String> {
    let msg = IpcMessage::ToggleLocalOnly { id, local_only };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
fn get_state(key: String) -> Result<Option<String>, String> {
    let msg = IpcMessage::GetState { key };
    match send_ipc_message(msg)? {
        IpcMessage::StateResponse(val) => Ok(val),
        IpcMessage::Log(err) => Err(err),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
fn set_state(key: String, value: String) -> Result<String, String> {
    let msg = IpcMessage::SetState { key, value };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn start_scan() -> Result<String, String> {
    let msg = IpcMessage::StartScan;
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn stop_scan() -> Result<String, String> {
    let msg = IpcMessage::StopScan;
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn get_discovered_devices() -> Result<Vec<(String, String, String, Vec<String>, u16)>, String> {
    let msg = IpcMessage::GetDiscovered;
    match send_ipc_message(msg)? {
        IpcMessage::DiscoveredResponse(list) => Ok(list),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn pair_with(node_id: String) -> Result<String, String> {
    let msg = IpcMessage::PairWith { node_id };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn manual_pair(ip: String, port: u16) -> Result<String, String> {
    let msg = IpcMessage::PairWithIp { ip, port };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn confirm_pairing(accepted: bool) -> Result<String, String> {
    let msg = IpcMessage::ConfirmPairing(accepted);
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn get_pairing_status() -> Result<(Option<String>, bool, bool, String, bool), String> {
    let msg = IpcMessage::GetPairingStatus;
    match send_ipc_message(msg)? {
        IpcMessage::PairingStatusResponse {
            pin,
            active,
            is_initiator,
            remote_label,
            silent,
        } => Ok((pin, active, is_initiator, remote_label, silent)),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn get_paired_devices() -> Result<Vec<(String, String, Option<TransportType>)>, String> {
    let msg = IpcMessage::GetPairedDevices;
    match send_ipc_message(msg)? {
        IpcMessage::PairedDevicesResponse(devices) => Ok(devices),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn unpair_device(node_id: String) -> Result<String, String> {
    let msg = IpcMessage::UnpairDevice { node_id };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn send_file(node_id: String, path: String) -> Result<String, String> {
    let msg = IpcMessage::SendFile { node_id, path };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn accept_file_transfer(transfer_id: String) -> Result<String, String> {
    let msg = IpcMessage::AcceptFileTransfer { transfer_id };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn reject_file_transfer(transfer_id: String) -> Result<String, String> {
    let msg = IpcMessage::RejectFileTransfer { transfer_id };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn get_file_transfer_history(limit: u32) -> Result<Vec<cdus_common::FileTransferRecord>, String> {
    let msg = IpcMessage::GetFileTransferHistory { limit };
    match send_ipc_message(msg)? {
        IpcMessage::FileTransferHistoryResponse(history) => Ok(common_history_to_tauri(history)),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn clear_finished_transfers() -> Result<String, String> {
    let msg = IpcMessage::ClearFinishedTransfers;
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

fn common_history_to_tauri(history: Vec<cdus_common::FileTransferRecord>) -> Vec<cdus_common::FileTransferRecord> {
    history // They are already the same type now!
}

#[tauri::command]
async fn start_benchmark(node_id: String) -> Result<String, String> {
    let msg = IpcMessage::StartBenchmark { node_id };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn get_qr_pairing_payload() -> Result<String, String> {
    let msg = IpcMessage::GetQrPairingPayload;
    match send_ipc_message(msg)? {
        IpcMessage::QrPairingPayloadResponse { payload } => Ok(payload),
        _ => Err("Failed to get QR payload".to_string()),
    }
}

#[tauri::command]
async fn pair_with_qr(payload: String) -> Result<String, String> {
    let msg = IpcMessage::PairWithQr { payload };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => {
            if msg.starts_with("Error") {
                Err(msg)
            } else {
                Ok(msg)
            }
        }
        _ => Err("Failed to initiate QR pairing".to_string()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            last_synced: Mutex::new(None),
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                window.hide().unwrap();
                #[cfg(target_os = "macos")]
                window.app_handle().set_activation_policy(tauri::ActivationPolicy::Accessory);
                api.prevent_close();
            }
            _ => {}
        })
        .setup(|app| {
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let status_i =
                MenuItem::with_id(app, "status", "Status: Checking...", false, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;

            let menu = Menu::with_items(app, &[&status_i, &separator, &quit_i])?;

            let tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            #[cfg(target_os = "macos")]
                            let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // Status and Tooltip update thread
            let status_handle = status_i.clone();
            let tray_handle = tray.clone();
            let app_handle = app.handle().clone();

            thread::spawn(move || loop {
                let online = check_agent_online();
                let label = if online {
                    "Status: Online (LAN)"
                } else {
                    "Status: Offline (Agent Disconnected)"
                };
                let _ = status_handle.set_text(label);

                // Update tooltip
                let state = app_handle.state::<AppState>();
                let last_synced = {
                    let ls = state.last_synced.lock().unwrap();
                    *ls
                };

                let tooltip = match last_synced {
                    Some(time) => {
                        let elapsed = time.elapsed().unwrap_or(Duration::from_secs(0)).as_secs();
                        if elapsed < 60 {
                            format!("CDUS - Last synced: {}s ago", elapsed)
                        } else {
                            format!("CDUS - Last synced: {}m ago", elapsed / 60)
                        }
                    }
                    None => "CDUS - No sync yet".to_string(),
                };
                let _ = tray_handle.set_tooltip(Some(tooltip));

                thread::sleep(Duration::from_secs(5));
            });

            // Agent Event Stream Listener
            let app_handle_events = app.handle().clone();
            thread::spawn(move || loop {
                if let Ok(mut stream) = LocalSocketStream::connect(get_socket_path()) {
                    let msg = IpcMessage::ListenEvents;
                    if let Ok(bytes) = serde_json::to_vec(&msg) {
                        if let Ok(_) = stream.write_all(&bytes) {
                            use std::io::BufRead;
                            use std::io::BufReader;
                            let reader = BufReader::new(stream);
                            for line in reader.lines() {
                                if let Ok(line) = line {
                                    if let Ok(event) = serde_json::from_str::<IpcMessage>(&line) {
                                        match event {
                                            IpcMessage::FileProgress(progress_event) => {
                                                use cdus_common::ProgressEvent;
                                                match progress_event {
                                                    ProgressEvent::IncomingRequest {
                                                        transfer_id,
                                                        node_id,
                                                        file_name,
                                                        total_bytes,
                                                        sender_label: _,
                                                    } => {
                                                        let _ = app_handle_events.emit(
                                                            "incoming-file-request",
                                                            (
                                                                node_id,
                                                                serde_json::json!({
                                                                    "file_hash": transfer_id,
                                                                    "file_name": file_name,
                                                                    "total_size": total_bytes,
                                                                }),
                                                            ),
                                                        );
                                                    }
                                                    ProgressEvent::Started {
                                                        transfer_id,
                                                        file_name: _,
                                                        total_bytes: _,
                                                        is_outgoing,
                                                    } => {
                                                        let event_name = if is_outgoing {
                                                            "file-transfer-progress"
                                                        } else {
                                                            "file-transfer-progress"
                                                        };
                                                        let _ = app_handle_events.emit(
                                                            event_name,
                                                            (transfer_id, 0.0),
                                                        );
                                                    }
                                                    ProgressEvent::Progress {
                                                        transfer_id,
                                                        bytes_confirmed,
                                                        total_bytes,
                                                    } => {
                                                        let progress = if total_bytes > 0 {
                                                            (bytes_confirmed as f32 / total_bytes as f32) * 100.0
                                                        } else {
                                                            0.0
                                                        };
                                                        let _ = app_handle_events.emit(
                                                            "file-transfer-progress",
                                                            (transfer_id, progress),
                                                        );
                                                    }
                                                    ProgressEvent::Complete {
                                                        transfer_id,
                                                        ..
                                                    } => {
                                                        let _ = app_handle_events
                                                            .emit("file-transfer-complete", transfer_id);
                                                    }
                                                    ProgressEvent::Failed {
                                                        transfer_id,
                                                        reason,
                                                    } => {
                                                        let _ = app_handle_events.emit(
                                                            "file-transfer-error",
                                                            (transfer_id, reason),
                                                        );
                                                    }
                                                }
                                            }
                                            IpcMessage::FileTransferProgress {
                                                transfer_id,
                                                progress,
                                            } => {
                                                let _ = app_handle_events.emit(
                                                    "file-transfer-progress",
                                                    (transfer_id, progress),
                                                );
                                            }
                                            IpcMessage::FileTransferComplete { transfer_id } => {
                                                let _ = app_handle_events
                                                    .emit("file-transfer-complete", transfer_id);
                                            }
                                            IpcMessage::FileTransferError { transfer_id, error } => {
                                                let _ = app_handle_events.emit(
                                                    "file-transfer-error",
                                                    (transfer_id, error),
                                                );
                                            }
                                            IpcMessage::ClipboardChanged { content, .. }
                                            | IpcMessage::SetClipboard { content, .. } => {
                                                let _ = app_handle_events
                                                    .emit("clipboard-updated", content);
                                            }
                                            IpcMessage::PeerDisconnected { node_id } => {
                                                let _ = app_handle_events
                                                    .emit("peer-disconnected", node_id);
                                            }
                                            IpcMessage::PeerConnected { node_id } => {
                                                let _ = app_handle_events
                                                    .emit("peer-connected", node_id);
                                            }
                                            IpcMessage::PairingResult { success, node_id, label } => {
                                                let _ = app_handle_events
                                                    .emit("pairing-result", (success, node_id, label));
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                thread::sleep(Duration::from_secs(2));
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            ping_agent,
            set_clipboard,
            get_clipboard_history,
            delete_clipboard_item,
            toggle_local_only,
            clear_clipboard_history,
            get_state,
            set_state,
            start_scan,
            stop_scan,
            get_discovered_devices,
            pair_with,
            manual_pair,
            confirm_pairing,
            get_pairing_status,
            get_paired_devices,
            unpair_device,
            send_file,
            accept_file_transfer,
            reject_file_transfer,
            get_file_transfer_history,
            clear_finished_transfers,
            start_benchmark,
            get_qr_pairing_payload,
            pair_with_qr,
            read_system_clipboard,
            broadcast_clipboard
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
