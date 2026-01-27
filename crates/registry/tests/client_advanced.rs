//! Advanced client behavior tests (chunked upload, mount, delete, referrers).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use containerregistry_image::{Digest, ImageIndex};
use containerregistry_registry::{Client, ClientConfig, Reference};

#[derive(Debug)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    #[allow(dead_code)]
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
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
    let mut body = Vec::new();
    let mut method = String::new();
    let mut path = String::new();

    if let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        let (head, rest) = buf.split_at(split + 4);
        let head_str = String::from_utf8_lossy(head);
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

        if let Some(len) = headers
            .get("content-length")
            .and_then(|v| v.parse::<usize>().ok())
        {
            body.extend_from_slice(rest);
            while body.len() < len {
                let mut more = vec![0u8; len - body.len()];
                let n = stream.read(&mut more).unwrap_or(0);
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&more[..n]);
            }
            body.truncate(len);
        }
    }

    RecordedRequest {
        method,
        path,
        headers,
        body,
    }
}

fn serve_requests(responses: Vec<String>) -> (String, Arc<Mutex<Vec<RecordedRequest>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("server addr");
    let recorded: Arc<Mutex<Vec<RecordedRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded_clone = recorded.clone();
    let handle = thread::spawn(move || {
        for response in responses {
            if let Ok((mut stream, _)) = listener.accept() {
                let req = read_request(&mut stream);
                recorded_clone.lock().unwrap().push(req);
                let _ = stream.write_all(response.as_bytes());
            }
        }
    });
    (format!("{}:{}", addr.ip(), addr.port()), recorded, handle)
}

fn http_response(status: &str, headers: Vec<(&str, String)>, body: &str) -> String {
    let mut resp = format!("HTTP/1.1 {}\r\n", status);
    for (k, v) in headers {
        resp.push_str(&format!("{}: {}\r\n", k, v));
    }
    resp.push_str("\r\n");
    resp.push_str(body);
    resp
}

#[tokio::test]
async fn test_put_blob_chunked_sequence() {
    let responses = vec![
        http_response(
            "202 Accepted",
            vec![
                ("Location", "/upload/uuid".into()),
                ("Content-Length", "0".into()),
            ],
            "",
        ),
        http_response(
            "202 Accepted",
            vec![
                ("Range", "0-9".into()),
                ("Content-Length", "0".into()),
            ],
            "",
        ),
        http_response(
            "202 Accepted",
            vec![
                ("Range", "0-19".into()),
                ("Content-Length", "0".into()),
            ],
            "",
        ),
        http_response(
            "202 Accepted",
            vec![
                ("Range", "0-25".into()),
                ("Content-Length", "0".into()),
            ],
            "",
        ),
        http_response("201 Created", vec![("Content-Length", "0".into())], ""),
    ];

    let (addr, recorded, handle) = serve_requests(responses);
    let reference: Reference = format!("{}/repo:tag", addr).parse().unwrap();
    let client = Client::with_config(ClientConfig::new().with_https(false)).unwrap();

    let data = b"abcdefghijklmnopqrstuvwxyz";
    let digest = client
        .put_blob_chunked(&reference, data, 10)
        .await
        .unwrap();
    assert_eq!(digest, Digest::sha256(data));

    let _ = handle.join();
    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 5);
    assert_eq!(requests[0].method, "POST");
    assert!(requests[0].path.contains("/v2/repo/blobs/uploads/"));
    assert_eq!(requests[1].method, "PATCH");
    assert_eq!(requests[1].headers.get("content-range"), Some(&"0-9".to_string()));
    assert_eq!(requests[2].headers.get("content-range"), Some(&"10-19".to_string()));
    assert_eq!(requests[3].headers.get("content-range"), Some(&"20-25".to_string()));
    assert_eq!(requests[4].method, "PUT");
}

#[tokio::test]
async fn test_mount_blob_success() {
    let digest: Digest =
        "sha256:1111111111111111111111111111111111111111111111111111111111111111"
            .parse()
            .unwrap();
    let response = http_response("201 Created", vec![("Content-Length", "0".into())], "");
    let (addr, recorded, handle) = serve_requests(vec![response]);
    let reference: Reference = format!("{}/repo:tag", addr).parse().unwrap();
    let client = Client::with_config(ClientConfig::new().with_https(false)).unwrap();

    let mounted = client.mount_blob(&reference, "other/repo", &digest).await.unwrap();
    assert!(mounted);

    let _ = handle.join();
    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert!(requests[0].path.contains("mount="));
    assert!(
        requests[0].path.contains("from=other%2Frepo")
            || requests[0].path.contains("from=other/repo")
    );
}

#[tokio::test]
async fn test_mount_blob_fallback_to_upload() {
    let digest: Digest =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222"
            .parse()
            .unwrap();
    let response = http_response("202 Accepted", vec![("Content-Length", "0".into())], "");
    let (addr, _recorded, handle) = serve_requests(vec![response]);
    let reference: Reference = format!("{}/repo:tag", addr).parse().unwrap();
    let client = Client::with_config(ClientConfig::new().with_https(false)).unwrap();

    let mounted = client.mount_blob(&reference, "other/repo", &digest).await.unwrap();
    assert!(!mounted);

    let _ = handle.join();
}

#[tokio::test]
async fn test_delete_manifest() {
    let response = http_response("202 Accepted", vec![("Content-Length", "0".into())], "");
    let (addr, recorded, handle) = serve_requests(vec![response]);
    let reference: Reference = format!("{}/repo:tag", addr).parse().unwrap();
    let client = Client::with_config(ClientConfig::new().with_https(false)).unwrap();

    client.delete_manifest(&reference).await.unwrap();

    let _ = handle.join();
    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "DELETE");
    assert!(requests[0].path.contains("/v2/repo/manifests/"));
}

#[tokio::test]
async fn test_delete_blob() {
    let digest: Digest =
        "sha256:3333333333333333333333333333333333333333333333333333333333333333"
            .parse()
            .unwrap();
    let response = http_response("202 Accepted", vec![("Content-Length", "0".into())], "");
    let (addr, recorded, handle) = serve_requests(vec![response]);
    let reference: Reference = format!("{}/repo:tag", addr).parse().unwrap();
    let client = Client::with_config(ClientConfig::new().with_https(false)).unwrap();

    client.delete_blob(&reference, &digest).await.unwrap();

    let _ = handle.join();
    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "DELETE");
    assert!(requests[0].path.contains("/v2/repo/blobs/"));
}

#[tokio::test]
async fn test_get_referrers() {
    let digest: Digest =
        "sha256:4444444444444444444444444444444444444444444444444444444444444444"
            .parse()
            .unwrap();
    let body = r#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[]}"#;
    let response = http_response(
        "200 OK",
        vec![
            ("Content-Type", "application/vnd.oci.image.index.v1+json".into()),
            ("Content-Length", body.len().to_string()),
        ],
        body,
    );
    let (addr, _recorded, handle) = serve_requests(vec![response]);
    let reference: Reference = format!("{}/repo:tag", addr).parse().unwrap();
    let client = Client::with_config(ClientConfig::new().with_https(false)).unwrap();

    let index: ImageIndex = client.get_referrers(&reference, &digest).await.unwrap();
    assert_eq!(index.media_type().as_str(), "application/vnd.oci.image.index.v1+json");

    let _ = handle.join();
}
