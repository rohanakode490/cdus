use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Manager,
};

use cdus_common::{IpcMessage, ClipboardEvent};
use interprocess::local_socket::LocalSocketStream;
use std::io::{Read, Write};
use std::time::{Duration, SystemTime};
use std::thread;
use std::sync::Mutex;

struct AppState {
    last_synced: Mutex<Option<SystemTime>>,
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

fn check_agent_online() -> bool {
    let socket_name = "/tmp/cdus-agent.sock";
    match LocalSocketStream::connect(socket_name) {
        Ok(mut stream) => {
            let msg = IpcMessage::Ping;
            if let Ok(bytes) = serde_json::to_vec(&msg) {
                if stream.write_all(&bytes).is_ok() {
                    let mut buffer = [0u8; 1024];
                    if let Ok(n) = stream.read(&mut buffer) {
                        if let Ok(response) = serde_json::from_slice::<IpcMessage>(&buffer[..n]) {
                            return response == IpcMessage::Pong;
                        }
                    }
                }
            }
            false
        }
        Err(_) => false,
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
fn set_clipboard(content: String) -> Result<String, String> {
    let socket_name = "/tmp/cdus-agent.sock";
    let mut stream = LocalSocketStream::connect(socket_name)
        .map_err(|e| format!("Failed to connect to agent: {}", e))?;

    let msg = IpcMessage::SetClipboard(content);
    let bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;

    let mut buffer = [0u8; 1024];
    let n = stream.read(&mut buffer).map_err(|e| e.to_string())?;
    let response: IpcMessage = serde_json::from_slice(&buffer[..n]).map_err(|e| e.to_string())?;

    Ok(format!("{:?}", response))
}

#[tauri::command]
fn get_clipboard_history(state: tauri::State<'_, AppState>, limit: u32) -> Result<Vec<ClipboardEvent>, String> {
    let socket_name = "/tmp/cdus-agent.sock";
    let mut stream = LocalSocketStream::connect(socket_name)
        .map_err(|e| format!("Failed to connect to agent: {}", e))?;

    let msg = IpcMessage::GetHistory { limit };
    let bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;

    let mut buffer = [0u8; 8192]; // Larger buffer for history
    let n = stream.read(&mut buffer).map_err(|e| e.to_string())?;
    let response: IpcMessage = serde_json::from_slice(&buffer[..n]).map_err(|e| e.to_string())?;

    match response {
        IpcMessage::HistoryResponse(history) => {
            if !history.is_empty() {
                let mut ls = state.last_synced.lock().unwrap();
                *ls = Some(SystemTime::now());
            }
            Ok(history)
        },
        IpcMessage::Log(err) => Err(err),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
fn get_state(key: String) -> Result<Option<String>, String> {
    let socket_name = "/tmp/cdus-agent.sock";
    let mut stream = LocalSocketStream::connect(socket_name)
        .map_err(|e| format!("Failed to connect to agent: {}", e))?;

    let msg = IpcMessage::GetState { key };
    let bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;

    let mut buffer = [0u8; 1024];
    let n = stream.read(&mut buffer).map_err(|e| e.to_string())?;
    let response: IpcMessage = serde_json::from_slice(&buffer[..n]).map_err(|e| e.to_string())?;

    match response {
        IpcMessage::StateResponse(val) => Ok(val),
        IpcMessage::Log(err) => Err(err),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
fn set_state(key: String, value: String) -> Result<String, String> {
    let socket_name = "/tmp/cdus-agent.sock";
    let mut stream = LocalSocketStream::connect(socket_name)
        .map_err(|e| format!("Failed to connect to agent: {}", e))?;

    let msg = IpcMessage::SetState { key, value };
    let bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;

    let mut buffer = [0u8; 1024];
    let n = stream.read(&mut buffer).map_err(|e| e.to_string())?;
    let response: IpcMessage = serde_json::from_slice(&buffer[..n]).map_err(|e| e.to_string())?;

    match response {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn start_scan() -> Result<String, String> {
    let socket_name = "/tmp/cdus-agent.sock";
    let mut stream = LocalSocketStream::connect(socket_name)
        .map_err(|e| format!("Failed to connect to agent: {}", e))?;

    let msg = IpcMessage::StartScan;
    let bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;

    let mut buffer = [0u8; 1024];
    let n = stream.read(&mut buffer).map_err(|e| e.to_string())?;
    let response: IpcMessage = serde_json::from_slice(&buffer[..n]).map_err(|e| e.to_string())?;

    match response {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn stop_scan() -> Result<String, String> {
    let socket_name = "/tmp/cdus-agent.sock";
    let mut stream = LocalSocketStream::connect(socket_name)
        .map_err(|e| format!("Failed to connect to agent: {}", e))?;

    let msg = IpcMessage::StopScan;
    let bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;

    let mut buffer = [0u8; 1024];
    let n = stream.read(&mut buffer).map_err(|e| e.to_string())?;
    let response: IpcMessage = serde_json::from_slice(&buffer[..n]).map_err(|e| e.to_string())?;

    match response {
        IpcMessage::Log(msg) => Ok(msg),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[tauri::command]
async fn get_discovered_devices() -> Result<Vec<(String, String, String)>, String> {
    let socket_name = "/tmp/cdus-agent.sock";
    let mut stream = LocalSocketStream::connect(socket_name)
        .map_err(|e| format!("Failed to connect to agent: {}", e))?;

    let msg = IpcMessage::GetDiscovered;
    let bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;

    let mut buffer = [0u8; 4096];
    let n = stream.read(&mut buffer).map_err(|e| e.to_string())?;
    let response: IpcMessage = serde_json::from_slice(&buffer[..n]).map_err(|e| e.to_string())?;

    match response {
        IpcMessage::DiscoveredResponse(list) => Ok(list),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState { last_synced: Mutex::new(None) })
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let status_i = MenuItem::with_id(app, "status", "Status: Checking...", false, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            
            let menu = Menu::with_items(app, &[&status_i, &separator, &quit_i])?;

            let tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(true)
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

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet, 
            ping_agent, 
            set_clipboard, 
            get_clipboard_history,
            get_state,
            set_state,
            start_scan,
            stop_scan,
            get_discovered_devices
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

