use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};

use cdus_common::{IpcMessage, ClipboardEvent};
use interprocess::local_socket::LocalSocketStream;
use std::io::{Read, Write};

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn ping_agent() -> Result<String, String> {
    let socket_name = "/tmp/cdus-agent.sock";
    let mut stream = LocalSocketStream::connect(socket_name)
        .map_err(|e| format!("Failed to connect to agent: {}", e))?;

    let msg = IpcMessage::Ping;
    let bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;

    let mut buffer = [0u8; 1024];
    let n = stream.read(&mut buffer).map_err(|e| e.to_string())?;
    let response: IpcMessage = serde_json::from_slice(&buffer[..n]).map_err(|e| e.to_string())?;

    Ok(format!("{:?}", response))
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
fn get_clipboard_history(limit: u32) -> Result<Vec<ClipboardEvent>, String> {
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
        IpcMessage::HistoryResponse(history) => Ok(history),
        IpcMessage::Log(err) => Err(err),
        _ => Err("Unexpected response from agent".to_string()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let status_i = MenuItem::with_id(app, "status", "Status: Online (LAN)", false, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            
            let menu = Menu::with_items(app, &[&status_i, &separator, &quit_i])?;

            let _tray = TrayIconBuilder::new()
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

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![greet, ping_agent, set_clipboard, get_clipboard_history])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
