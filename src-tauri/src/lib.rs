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
    history_items: Mutex<Vec<String>>,
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
fn search(query: String) -> Result<Vec<cdus_common::SearchResult>, String> {
    let msg = IpcMessage::Search { query };
    match send_ipc_message(msg)? {
        IpcMessage::SearchResponse(results) => Ok(results),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
fn submit_feedback(text: String, attach_logs: bool) -> Result<String, String> {
    let msg = IpcMessage::SubmitFeedback { text, attach_logs };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
fn set_telemetry_opt_in(opt_in: bool) -> Result<String, String> {
    let msg = IpcMessage::SetTelemetryOptIn { opt_in };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
fn get_telemetry_opt_in() -> Result<bool, String> {
    let msg = IpcMessage::GetTelemetryOptIn;
    match send_ipc_message(msg)? {
        IpcMessage::TelemetryOptInResponse(opt_in) => Ok(opt_in),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn get_audit_logs(limit: u32) -> Result<Vec<cdus_common::AuditLogRecord>, String> {
    let msg = IpcMessage::GetAuditLogs { limit };
    match send_ipc_message(msg)? {
        IpcMessage::AuditLogsResponse(logs) => Ok(logs),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn clear_audit_logs() -> Result<String, String> {
    let msg = IpcMessage::ClearAuditLogs;
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn append_audit_log(event_type: String, content: String) -> Result<String, String> {
    let msg = IpcMessage::AppendAuditLog { event_type, content };
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
async fn cancel_file_transfer(transfer_id: String) -> Result<String, String> {
    let msg = IpcMessage::CancelFileTransfer { transfer_id };
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
        IpcMessage::Log(msg) => {
            if msg.starts_with("Error") {
                Err(msg)
            } else {
                Ok(msg)
            }
        }
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn delete_file_transfer(transfer_id: String) -> Result<String, String> {
    let msg = IpcMessage::DeleteFileTransfer { transfer_id };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => {
            if msg.starts_with("Error") {
                Err(msg)
            } else {
                Ok(msg)
            }
        }
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn delete_file_permanently(transfer_id: String) -> Result<String, String> {
    let msg = IpcMessage::DeleteFilePermanently { transfer_id };
    match send_ipc_message(msg)? {
        IpcMessage::Log(msg) => {
            if msg.starts_with("Error") {
                Err(msg)
            } else {
                Ok(msg)
            }
        }
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

#[tauri::command]
async fn open_file_location(app: tauri::AppHandle, transfer_id: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    // 1. Get the file transfer history from the agent daemon (up to 1000 items)
    let msg = IpcMessage::GetFileTransferHistory { limit: 1000 };
    let history = match send_ipc_message(msg)? {
        IpcMessage::FileTransferHistoryResponse(history) => history,
        _ => return Err("Unexpected response from agent".to_string()),
    };

    // 2. Find the transfer record with the matching ID
    let record = history
        .into_iter()
        .find(|r| r.transfer_id == transfer_id)
        .ok_or_else(|| "File transfer record not found".to_string())?;

    // 3. Handle edge cases: check status
    if record.status != "complete" {
        return Err("Cannot open location: Transfer is not complete".to_string());
    }

    // Check if path is empty or benchmark
    if record.file_path.is_empty() || record.file_path == "/dev/null" {
        return Err("Cannot open location: This is a benchmark transfer".to_string());
    }

    // Check if the physical file exists
    let path = std::path::Path::new(&record.file_path);
    if !path.exists() {
        return Err("The file could not be found. It may have been moved or deleted.".to_string());
    }

    // 4. Reveal the item in the file manager
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| format!("Failed to open file location: {}", e))?;

    Ok(())
}


fn update_tray_menu(app: &tauri::AppHandle) -> Result<(), String> {
    let history = match send_ipc_message(IpcMessage::GetHistory { limit: 5 }) {
        Ok(IpcMessage::HistoryResponse(h)) => h,
        _ => return Err("Failed to get clipboard history from agent".to_string()),
    };

    let state = app.state::<AppState>();
    let mut history_contents = Vec::new();
    for item in &history {
        history_contents.push(item.content.clone());
    }
    {
        let mut items = state.history_items.lock().unwrap();
        *items = history_contents;
    }

    let tray = match app.tray_by_id("main") {
        Some(t) => t,
        None => return Err("Tray icon not found".to_string()),
    };

    let mut menu_items: Vec<Box<dyn tauri::menu::IsMenuItem<tauri::Wry>>> = Vec::new();

    let online = check_agent_online();
    let status_label = if online {
        "Status: Online (LAN)"
    } else {
        "Status: Offline"
    };
    let status_i = MenuItem::with_id(app, "status", status_label, false, None::<&str>).map_err(|e| e.to_string())?;
    menu_items.push(Box::new(status_i));

    let separator1 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    menu_items.push(Box::new(separator1));

    if history.is_empty() {
        let empty_i = MenuItem::with_id(app, "empty_history", "Clipboard history is empty", false, None::<&str>).map_err(|e| e.to_string())?;
        menu_items.push(Box::new(empty_i));
    } else {
        for (idx, item) in history.iter().enumerate() {
            let mut display_text = item.content.replace("\n", " ");
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&item.content) {
                if let Some(typ) = json_val.get("type").and_then(|v| v.as_str()) {
                    match typ {
                        "image" => {
                            display_text = "[Image Clipboard]".to_string();
                        }
                        "url" => {
                            if let Some(url_val) = json_val.get("url").and_then(|v| v.as_str()) {
                                display_text = format!("🌐 {}", url_val);
                            }
                        }
                        _ => {}
                    }
                }
            }
            if display_text.len() > 30 {
                display_text = format!("{}...", &display_text[..30]);
            }
            let history_i = MenuItem::with_id(
                app,
                format!("history_{}", idx),
                display_text,
                true,
                None::<&str>,
            ).map_err(|e| e.to_string())?;
            menu_items.push(Box::new(history_i));
        }
    }

    let separator2 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    menu_items.push(Box::new(separator2));

    let show_i = MenuItem::with_id(app, "show_main", "Show Main Window", true, None::<&str>).map_err(|e| e.to_string())?;
    menu_items.push(Box::new(show_i));

    let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>).map_err(|e| e.to_string())?;
    menu_items.push(Box::new(quit_i));

    let refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = menu_items.iter().map(|item| item.as_ref()).collect();
    let new_menu = Menu::with_items(app, &refs).map_err(|e| e.to_string())?;

    tray.set_menu(Some(new_menu)).map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            last_synced: Mutex::new(None),
            history_items: Mutex::new(Vec::new()),
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())

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
                .on_menu_event(|app, event| {
                    let id = event.id.as_ref();
                    if id == "quit" {
                        app.exit(0);
                    } else if id == "show_main" {
                        if let Some(window) = app.get_webview_window("main") {
                            #[cfg(target_os = "macos")]
                            let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    } else if id.starts_with("history_") {
                        if let Ok(idx) = id.strip_prefix("history_").unwrap().parse::<usize>() {
                            let state = app.state::<AppState>();
                            let items = state.history_items.lock().unwrap();
                            if idx < items.len() {
                                let content = items[idx].clone();
                                #[cfg(not(target_os = "android"))]
                                {
                                    use arboard::Clipboard;
                                    if let Ok(mut cb) = Clipboard::new() {
                                        let mut text_to_copy = content.clone();
                                        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&content) {
                                            if let Some(typ) = json_val.get("type").and_then(|v| v.as_str()) {
                                                match typ {
                                                    "url" => {
                                                        if let Some(url_val) = json_val.get("url").and_then(|v| v.as_str()) {
                                                            text_to_copy = url_val.to_string();
                                                        }
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                        let _ = cb.set_text(text_to_copy);
                                    }
                                }
                            }
                        }
                    }
                })
                .build(app)?;

            // Status and Tooltip update thread
            let tray_handle = tray.clone();
            let app_handle = app.handle().clone();

            thread::spawn(move || loop {
                let _ = update_tray_menu(&app_handle);

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
                                                let _ = update_tray_menu(&app_handle_events);
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
            search,
            submit_feedback,
            set_telemetry_opt_in,
            get_telemetry_opt_in,
            ping_agent,
            set_clipboard,
            get_clipboard_history,
            delete_clipboard_item,
            toggle_local_only,
            clear_clipboard_history,
            get_audit_logs,
            clear_audit_logs,
            append_audit_log,
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
            cancel_file_transfer,
            get_file_transfer_history,
            clear_finished_transfers,
            delete_file_transfer,
            delete_file_permanently,
            start_benchmark,
            get_qr_pairing_payload,
            pair_with_qr,
            read_system_clipboard,
            broadcast_clipboard,
            open_file_location
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
