// Declaring the app's commands makes Tauri require an explicit
// `allow-<command>` permission for each one, instead of exposing every
// command to every window by default.
const COMMANDS: &[&str] = &[
    "ui_ready",
    "app_info",
    "system_info",
    "get_settings",
    "update_settings",
    "diagnostics",
    "recent_logs",
    "open_data_dir",
    "open_logs_dir",
    "list_versions",
    "install_version",
    "cancel_download",
    "download_queue",
    "download_history",
    "clear_download_history",
    "detect_java",
    "set_instance_java",
    "list_instances",
    "get_instance",
    "create_instance",
    "update_instance",
    "delete_instance",
    "open_instance_dir",
    "set_instance_jvm_args",
    "loader_versions",
    "set_instance_loader_version",
    "set_instance_game_dir",
    "pick_folder",
    "modrinth_search",
    "modrinth_project",
    "modrinth_categories",
    "open_modrinth_page",
    "show_snap_layouts",
    "data_folder_info",
    "relocate_data",
    "check_for_update",
    "install_update",
    "mod_install_plan",
    "mod_install",
    "list_mods",
    "set_mod_enabled",
    "remove_mod",
    "check_mod_updates",
    "update_mods",
    "import_local_mod",
    "account",
    "sign_in",
    "cancel_sign_in",
    "list_accounts",
    "check_active_account",
    "switch_account",
    "remove_account",
    "running_instances",
    "launch_instance",
    "stop_instance",
];

fn main() {
    // The update channel is compiled in (commands/update.rs).
    println!("cargo:rerun-if-env-changed=NIGHT_CLIENT_UPDATE_PUBKEY");
    println!("cargo:rerun-if-env-changed=NIGHT_CLIENT_UPDATE_ENDPOINT");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
