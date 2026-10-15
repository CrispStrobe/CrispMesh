#[tauri::command]
fn desktop_capabilities() -> &'static str {
    "Desktop shell · reconstruction unavailable"
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![desktop_capabilities])
        .run(tauri::generate_context!())
        .expect("error while running CrispMesh");
}
