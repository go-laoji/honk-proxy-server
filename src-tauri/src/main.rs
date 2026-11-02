// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().skip(1).collect::<Vec<_>>() == ["--build-info"] {
        let info = hps_domain::compiled_build_info(
            "hps-desktop",
            hps_domain::VersionInfo {
                service: "honk-proxy-server".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                api_version: hps_domain::API_VERSION.into(),
                web_build_id: Some(hps_web_assets::BUILD_ID.into()),
            },
        );
        println!(
            "{}",
            serde_json::to_string(&info).expect("compiled metadata")
        );
        return;
    }
    honk_proxy_server_lib::run()
}
