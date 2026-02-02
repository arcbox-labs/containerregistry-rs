//! Bearer auth challenge end-to-end test.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use containerregistry_image::Digest;
use containerregistry_registry::{Client, ClientConfig, Reference};

fn read_request(stream: &mut TcpStream) -> (String, String, HashMap<String, String>) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = stream.read(&mut tmp).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }

    let mut headers = HashMap::new();
    let mut method = String::new();
    let mut path = String::new();
    let head_str = String::from_utf8_lossy(&buf);
    let mut lines = head_str.split("\r\n");
    if let Some(request_line) = lines.next() {
        let mut parts = request_line.split_whitespace();
        method = parts.next().unwrap_or("").to_string();
        path = parts.next().unwrap_or("").to_string();
    }
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    (method, path, headers)
}

fn http_response(status: &str, headers: Vec<(&str, String)>, body: &str) -> String {
    let mut resp = format!("HTTP/1.1 {}\r\n", status);
    resp.push_str("Connection: close\r\n");
    for (k, v) in headers {
        resp.push_str(&format!("{}: {}\r\n", k, v));
    }
    resp.push_str("\r\n");
    resp.push_str(body);
    resp
}

#[tokio::test]
async fn test_bearer_auth_flow() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().unwrap();
    let served = Arc::new(Mutex::new(0usize));
    let served_clone = served.clone();

    let handle = thread::spawn(move || {
        while *served_clone.lock().unwrap() < 3 {
            if let Ok((mut stream, _)) = listener.accept() {
                let (_method, path, headers) = read_request(&mut stream);
                let mut served_guard = served_clone.lock().unwrap();
                *served_guard += 1;

                if path.starts_with("/token") {
                    let body = r#"{"token":"token123"}"#;
                    let response = http_response(
                        "200 OK",
                        vec![
                            ("Content-Type", "application/json".into()),
                            ("Content-Length", body.len().to_string()),
                        ],
                        body,
                    );
                    let _ = stream.write_all(response.as_bytes());
                    continue;
                }

                if path.starts_with("/v2/repo/manifests/latest") {
                    let auth = headers
                        .get("authorization")
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    if auth == "Bearer token123" {
                        let body = r#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"mediaType":"application/vnd.oci.image.config.v1+json","digest":"sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a","size":2},"layers":[]}"#;
                        let digest = Digest::sha256(body.as_bytes());
                        let response = http_response(
                            "200 OK",
                            vec![
                                (
                                    "Content-Type",
                                    "application/vnd.oci.image.manifest.v1+json".into(),
                                ),
                                ("Docker-Content-Digest", digest.to_string()),
                                ("Content-Length", body.len().to_string()),
                            ],
                            body,
                        );
                        let _ = stream.write_all(response.as_bytes());
                        continue;
                    }

                    let challenge = format!(
                        "Bearer realm=\"http://{}/token\",service=\"registry\",scope=\"repository:repo:pull\"",
                        addr
                    );
                    let response = http_response(
                        "401 Unauthorized",
                        vec![
                            ("WWW-Authenticate", challenge),
                            ("Content-Length", "0".into()),
                        ],
                        "",
                    );
                    let _ = stream.write_all(response.as_bytes());
                    continue;
                }

                let response =
                    http_response("404 Not Found", vec![("Content-Length", "0".into())], "");
                let _ = stream.write_all(response.as_bytes());
            }
        }
    });

    let reference: Reference = format!("{}/repo:latest", addr).parse().unwrap();
    let client =
        Client::with_config(ClientConfig::new().with_https(false).with_retries(1)).unwrap();

    let result = client.get_manifest(&reference).await;
    assert!(result.is_ok(), "expected bearer auth flow to succeed");

    let _ = handle.join();
}
