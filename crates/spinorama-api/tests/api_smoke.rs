//! End-to-end smoke test: keys CLI, auth, docs, and one download.
//!
//! Spawns the real binary on a loopback port against a minimal
//! fixture tree; no Python involved.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const PORT: u16 = 18231;

fn fixture() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("spinorama-api-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets")).expect("assets");
    std::fs::create_dir_all(dir.join("speakers/Test Speaker/ASR/asr")).expect("spin");
    std::fs::write(
        dir.join("assets/metadata.json"),
        r#"{"Test Speaker":{"brand":"Test","model":"Speaker","type":"passive","price":"1","shape":"bookshelves","default_measurement":"asr","measurements":{"asr":{"origin":"ASR","format":"klippel","review_published":"20240301"},"eac":{"origin":"ErinsAudioCorner","format":"klippel"}}},"Old Speaker":{"brand":"Old","model":"Speaker","type":"passive","price":"1","shape":"bookshelves","default_measurement":"asr","measurements":{"asr":{"origin":"ASR","format":"klippel","review_published":"20230501"}}}}"#,
    )
    .expect("metadata");
    std::fs::write(
        dir.join("speakers/Test Speaker/ASR/asr/CEA2034.json"),
        r#"{"data":[{"name":"On Axis","x":[100.0,1000.0],"y":[80.0,85.0]}]}"#,
    )
    .expect("figure");
    dir
}

fn run_keys(dir: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_spinorama-api"))
        .args(["keys"])
        .args(args)
        .env("SPINORAMA_API_KEY_DB", dir.join("keys.sqlite3"))
        .output()
        .expect("keys CLI");
    assert!(
        output.status.success(),
        "keys {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn request(path: &str, token: Option<&str>) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", PORT)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    stream
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n{auth}\r\n").as_bytes())
        .expect("write");
    let mut raw = String::new();
    stream.read_to_string(&mut raw).expect("read");
    let status: u16 = raw
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
    (status, body)
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn api_smoke() {
    let dir = fixture();
    run_keys(&dir, &["init"]);
    let issued = run_keys(
        &dir,
        &[
            "issue",
            "--email",
            "a@example.org",
            "--name",
            "A",
            "--purpose",
            "Test",
        ],
    );
    let token = issued
        .split("API key (shown only once): ")
        .nth(1)
        .expect("token in output")
        .trim()
        .to_owned();
    let key_id = issued
        .split("Key ID: ")
        .nth(1)
        .expect("key id")
        .split_whitespace()
        .next()
        .expect("key id value")
        .to_owned();

    let child = Command::new(env!("CARGO_BIN_EXE_spinorama-api"))
        .arg("serve")
        .env("SPINORAMA_API_KEY_DB", dir.join("keys.sqlite3"))
        .env("SPINORAMA_API_FILES", &dir)
        .env("SPINORAMA_API_BIND", format!("tcp:127.0.0.1:{PORT}"))
        .env("SPINORAMA_FINGERPRINT_SECRET", "test-secret")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let _server = Server(child);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(("127.0.0.1", PORT)).is_ok() {
            break;
        }
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(50));
    }

    assert_eq!(request("/docs", None).0, 200);
    assert_eq!(request("/openapi.json", None).0, 200);
    assert_eq!(request("/v1/brands", None).0, 401);
    assert_eq!(request("/v1/brands", Some("wrong")).0, 401);

    let (status, body) = request("/v1/brands", Some(&token));
    assert_eq!(status, 200);
    assert!(body.contains("\"Test\""), "brands: {body}");

    let (status, body) = request("/v1/speaker/Test%20Speaker/metadata", Some(&token));
    assert_eq!(status, 200);
    assert!(body.contains("\"asr\""), "metadata: {body}");
    assert!(!body.contains("\"eac\""), "EAC must be filtered: {body}");

    let (status, body) = request(
        "/v1/speaker/Test%20Speaker/version/asr/measurements/CEA2034",
        Some(&token),
    );
    assert_eq!(status, 200);
    let outer: serde_json::Value = serde_json::from_str(&body).expect("outer json");
    let inner: serde_json::Value =
        serde_json::from_str(outer[0].as_str().expect("text payload")).expect("figure json");
    // Fingerprint must move y by at most 0.05 dB but not leave it pristine.
    let y = inner["data"][0]["y"][1].as_f64().expect("y");
    assert!(
        (y - 85.0).abs() > 0.0 && (y - 85.0).abs() <= 0.05,
        "fingerprinted y={y}"
    );

    // Demo prefix: no key, only the fixed ASR/2024 set, pristine data.
    let (status, body) = request("/demo/v1/speakers", None);
    assert_eq!(status, 200);
    assert!(body.contains("Test Speaker"), "demo speakers: {body}");
    assert!(
        !body.contains("Old Speaker"),
        "2023 speaker excluded: {body}"
    );
    let (status, body) = request("/demo/v1/brands", None);
    assert_eq!(status, 200);
    assert!(body.contains("\"Test\""), "demo brands: {body}");
    assert!(!body.contains("Old"), "demo brands: {body}");
    let (status, body) = request("/demo/v1/speaker/Test%20Speaker/metadata", None);
    assert_eq!(status, 200);
    assert!(body.contains("\"asr\""), "demo metadata: {body}");
    assert!(
        !body.contains("\"eac\""),
        "demo metadata is ASR-only: {body}"
    );
    let (_status, body) = request("/demo/v1/speaker/Old%20Speaker/metadata", None);
    assert!(body.contains("error"), "non-demo speaker rejected: {body}");
    let (status, body) = request(
        "/demo/v1/speaker/Test%20Speaker/version/asr/measurements/CEA2034",
        None,
    );
    assert_eq!(status, 200);
    let outer: serde_json::Value = serde_json::from_str(&body).expect("demo outer json");
    let inner: serde_json::Value =
        serde_json::from_str(outer[0].as_str().expect("demo text payload")).expect("demo figure");
    assert_eq!(
        inner["data"][0]["y"][1].as_f64(),
        Some(85.0),
        "demo data must be pristine (no fingerprint)"
    );
    let (_status, body) = request(
        "/demo/v1/speaker/Test%20Speaker/version/eac/measurements/CEA2034",
        None,
    );
    assert!(
        body.contains("error"),
        "demo rejects non-ASR version: {body}"
    );
    assert_eq!(request("/demo/v1/headphones", None).0, 404);
    assert_eq!(request("/demo/v1/speakers", Some("wrong")).0, 200);

    let listed = run_keys(&dir, &["list"]);
    assert!(!listed.contains(&token), "list must not leak the token");
    run_keys(&dir, &["revoke", &key_id]);
    assert_eq!(request("/v1/brands", Some(&token)).0, 401);

    let _ = std::fs::remove_dir_all(&dir);
}
