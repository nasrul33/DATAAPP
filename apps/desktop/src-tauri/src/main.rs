fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("Teratai desktop runtime failed to start");
}
