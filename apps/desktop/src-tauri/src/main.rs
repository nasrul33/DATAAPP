mod job_commands;
mod project_commands;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(project_commands::ProjectSession::default())
        .invoke_handler(tauri::generate_handler![
            project_commands::project_create,
            project_commands::project_open,
            project_commands::project_validate,
            project_commands::project_current,
            project_commands::project_close,
            job_commands::job_get,
            job_commands::job_list,
            job_commands::job_cancel,
        ])
        .run(tauri::generate_context!())
        .expect("Teratai desktop runtime failed to start");
}
