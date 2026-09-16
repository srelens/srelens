use tauri::{AppHandle, Manager, WebviewWindowBuilder, WebviewUrl};

#[tauri::command]
pub async fn open_context_window(app: AppHandle, context_id: String) -> Result<(), String> {
    let hex_id = context_id.as_bytes().iter().map(|b| format!("{:02x}", b)).collect::<String>();
    let label = format!("ctx-{}", hex_id);
    
    if app.get_webview_window(&label).is_some() {
        // Window already exists, just focus it
        if let Some(win) = app.get_webview_window(&label) {
            let _ = win.unminimize();
            let _ = win.show();
            let _ = win.set_focus();
        }
        return Ok(());
    }

    // Percent-encode everything that could break the query string.
    // `URLSearchParams.get` on the frontend decodes this correctly.
    let encoded_id: String = context_id
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect();
    
    let url = WebviewUrl::App(format!("index.html?context={}", encoded_id).into());
    
    let builder = WebviewWindowBuilder::new(&app, &label, url)
        .title("srelens")
        .inner_size(1024.0, 768.0)
        .min_inner_size(640.0, 480.0)
        .center();

    builder.build().map_err(|e| e.to_string())?;
    
    Ok(())
}
