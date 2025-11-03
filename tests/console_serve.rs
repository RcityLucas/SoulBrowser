use reqwest::blocking::Client;
use serde_json::Value;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

fn find_free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind probe port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn wait_for_health(client: &Client, port: u16) {
    let url = format!("http://127.0.0.1:{}/health", port);
    for _ in 0..60 {
        if let Ok(resp) = client.get(&url).send() {
            if resp.status().is_success() {
                return;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("console server did not become ready on port {}", port);
}

#[test]
fn serve_api_returns_fixture_payload() {
    let fixture_path = PathBuf::from("tests/fixtures/console_perception_fixture.json")
        .canonicalize()
        .expect("fixture exists");
    let port = find_free_port();

    let bin = assert_cmd::cargo::cargo_bin!("soulbrowser");
    let mut child = Command::new(bin)
        .env(
            "SOULBROWSER_CONSOLE_FIXTURE",
            fixture_path.to_str().expect("unicode path"),
        )
        .args(["--metrics-port", "0", "serve", "--port", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn serve");

    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client");

    wait_for_health(&client, port);

    let response = client
        .post(format!("http://127.0.0.1:{}/api/perceive", port))
        .json(&serde_json::json!({
            "url": "https://example.com",
            "mode": "all",
            "screenshot": true,
            "insights": true
        }))
        .send()
        .expect("request success");

    assert!(response.status().is_success());
    let body: Value = response.json().expect("json body");

    assert_eq!(body["success"].as_bool(), Some(true));
    assert_eq!(body["stdout"].as_str(), Some("fixture stdout"));
    assert!(body["screenshot_base64"].as_str().unwrap().len() > 16);

    let perception = body["perception"].as_object().expect("perception object");
    assert_eq!(
        perception["structural"]["dom_node_count"].as_u64(),
        Some(42)
    );
    assert_eq!(
        perception["visual"]["dominant_colors"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        perception["semantic"]["content_type"].as_str(),
        Some("Portal")
    );
    assert_eq!(perception["insights"].as_array().unwrap().len(), 2);

    let _ = child.kill();
    let _ = child.wait();
}
