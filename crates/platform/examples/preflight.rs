fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&hps_platform::preflight()).expect("preflight JSON")
    );
}
