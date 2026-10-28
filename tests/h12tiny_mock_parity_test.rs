use async_net::TcpListener;
use h12tiny::runtime::{BoxExecutor, BoxSendFuture, FnExecutor};
use h12tiny::server::serve;
use h12tiny::util::{boxed_body, bytes_body, collect_bytes_limited, empty_body};
use h12tiny::web::{Method, Request, Response, Router, State, StatusCode};
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::oneshot;

const BODY_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Default)]
struct MockS3 {
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    requests: Arc<Mutex<Vec<String>>>,
    signed: Arc<AtomicUsize>,
}

impl MockS3 {
    fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("mock store is not poisoned")
            .clone()
    }

    fn signed_count(&self) -> usize {
        self.signed.load(Ordering::Relaxed)
    }
}

fn xml(status: StatusCode, body: String) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/xml")
        .body(boxed_body(bytes_body(body)))
        .expect("static response parts are valid")
}

fn s3_error(status: StatusCode, code: &str, message: &str) -> Response {
    xml(
        status,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <Error><Code>{code}</Code><Message>{message}</Message></Error>"
        ),
    )
}

fn list_objects(store: &MockS3, prefix: &str) -> Response {
    let objects = store.objects.lock().expect("mock store is not poisoned");
    let matches: Vec<(&String, &Vec<u8>)> = objects
        .iter()
        .filter(|(key, _)| key.starts_with(prefix))
        .collect();
    let mut contents = String::new();
    for (key, value) in &matches {
        contents.push_str(&format!(
            "<Contents><Key>{key}</Key>\
             <LastModified>2026-01-01T00:00:00.000Z</LastModified>\
             <ETag>\"mock\"</ETag><Size>{}</Size>\
             <StorageClass>STANDARD</StorageClass></Contents>",
            value.len()
        ));
    }
    xml(
        StatusCode::OK,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
             <Name>kache-test</Name><Prefix>{prefix}</Prefix><KeyCount>{}</KeyCount>\
             <MaxKeys>1000</MaxKeys><IsTruncated>false</IsTruncated>{contents}</ListBucketResult>",
            matches.len()
        ),
    )
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => output.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                match (hex_digit(bytes[index + 1]), hex_digit(bytes[index + 2])) {
                    (Some(high), Some(low)) => {
                        output.push(high << 4 | low);
                        index += 2;
                    }
                    _ => output.push(b'%'),
                }
            }
            byte => output.push(byte),
        }
        index += 1;
    }
    String::from_utf8(output).expect("query fixtures are valid UTF-8")
}

fn parse_query(query: &str) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = match pair.split_once('=') {
            Some(split) => split,
            None => (pair, ""),
        };
        params.insert(decode_component(raw_key), decode_component(raw_value));
    }
    params
}

async fn s3_handler(State(store): State<MockS3>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    store
        .requests
        .lock()
        .expect("mock store is not poisoned")
        .push(format!("{} {}", parts.method, parts.uri));
    let authorized = parts
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.starts_with("AWS4-HMAC-SHA256 ")
                && value.contains("Credential=test-access-key/")
                && value.contains("SignedHeaders=")
                && value.contains("Signature=")
        })
        && parts.headers.contains_key("x-amz-date")
        && parts.headers.contains_key("x-amz-content-sha256");
    if !authorized {
        return s3_error(
            StatusCode::FORBIDDEN,
            "SignatureDoesNotMatch",
            "required SigV4 headers are missing",
        );
    }
    store.signed.fetch_add(1, Ordering::Relaxed);
    let query = parse_query(parts.uri.query().unwrap_or_default());
    if (parts.uri.path() == "/kache-test" || parts.uri.path() == "/kache-test/")
        && parts.method == Method::GET
        && query.get("list-type").is_some_and(|value| value == "2")
    {
        return list_objects(&store, query.get("prefix").map_or("", String::as_str));
    }
    let Some(key) = parts.uri.path().strip_prefix("/kache-test/") else {
        return s3_error(StatusCode::NOT_FOUND, "NoSuchBucket", "bucket not found");
    };
    let body = match collect_bytes_limited(body, BODY_LIMIT).await {
        Ok(body) => body,
        Err(_) => {
            return s3_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "EntityTooLarge",
                "request body exceeds the 16 MiB limit",
            );
        }
    };
    if parts.method == Method::PUT {
        let create_only = parts
            .headers
            .get("if-none-match")
            .and_then(|value| value.to_str().ok())
            == Some("*");
        let mut objects = store.objects.lock().expect("mock store is not poisoned");
        if create_only && objects.contains_key(key) {
            return s3_error(
                StatusCode::PRECONDITION_FAILED,
                "PreconditionFailed",
                "object already exists",
            );
        }
        objects.insert(key.to_string(), body.to_vec());
        return Response::builder()
            .status(StatusCode::OK)
            .header("etag", "\"mock\"")
            .body(boxed_body(empty_body()))
            .expect("static response parts are valid");
    }
    if parts.method == Method::HEAD {
        let objects = store.objects.lock().expect("mock store is not poisoned");
        let Some(value) = objects.get(key) else {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(boxed_body(empty_body()))
                .expect("static response parts are valid");
        };
        return Response::builder()
            .status(StatusCode::OK)
            .header("content-length", value.len().to_string())
            .header("etag", "\"mock\"")
            .body(boxed_body(empty_body()))
            .expect("static response parts are valid");
    }
    if parts.method == Method::GET {
        let objects = store.objects.lock().expect("mock store is not poisoned");
        let Some(value) = objects.get(key) else {
            return s3_error(StatusCode::NOT_FOUND, "NoSuchKey", "object not found");
        };
        return Response::builder()
            .status(StatusCode::OK)
            .header("content-length", value.len().to_string())
            .header("content-type", "application/octet-stream")
            .header("etag", "\"mock\"")
            .body(boxed_body(bytes_body(value.clone())))
            .expect("static response parts are valid");
    }
    s3_error(
        StatusCode::METHOD_NOT_ALLOWED,
        "MethodNotAllowed",
        "unsupported method",
    )
}

struct WireResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl WireResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value.as_str()))
    }
}

fn parse_response(raw: &[u8]) -> WireResponse {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response carries a header block");
    let (head, body) = raw.split_at(split);
    let head = std::str::from_utf8(head).expect("response head is ASCII");
    let mut lines = head.split("\r\n");
    let status_line = lines.next().expect("response carries a status line");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .expect("status line carries a code")
        .parse()
        .expect("status code is numeric");
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':').expect("header line carries a colon");
        headers.push((name.trim().to_lowercase(), value.trim().to_string()));
    }
    WireResponse {
        status,
        headers,
        body: body[4..].to_vec(),
    }
}

async fn round_trip(
    port: u16,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> WireResponse {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .expect("mock server accepts connections");
    let mut head = format!(
        "{method} {target} HTTP/1.1\r\nhost: 127.0.0.1\r\nconnection: close\r\ncontent-length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .await
        .expect("request head is writable");
    stream
        .write_all(body)
        .await
        .expect("request body is writable");
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .await
        .expect("response is readable");
    parse_response(&raw)
}

async fn start_server(
    store: MockS3,
) -> (
    u16,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock server binds loopback");
    let port = listener
        .local_addr()
        .expect("bound listener has an address")
        .port();
    let executor = BoxExecutor::new(FnExecutor::new(|future: BoxSendFuture| {
        tokio::spawn(future);
    }));
    let app = Router::<MockS3>::new()
        .fallback(s3_handler)
        .with_state(store);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        serve(listener, app, executor)
            .shutdown_on(async move {
                let _ = shutdown_rx.await;
            })
            .await
    });
    (port, shutdown_tx, server)
}

fn signed_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "authorization",
            "AWS4-HMAC-SHA256 Credential=test-access-key/20260101/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=deadbeef",
        ),
        ("x-amz-date", "20260101T000000Z"),
        ("x-amz-content-sha256", "UNSIGNED-PAYLOAD"),
    ]
}

async fn shutdown_cleanly(
    shutdown: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let _ = shutdown.send(());
    server
        .await
        .expect("server task joins")
        .expect("server shuts down cleanly");
}

#[tokio::test]
async fn put_get_head_round_trip_with_conditional_create() {
    let store = MockS3::default();
    let (port, shutdown, server) = start_server(store.clone()).await;
    let object = "/kache-test/artifacts/v3/packs/abc.tar.zst";
    let payload = b"pack-bytes";
    let signed = signed_headers();

    let put = round_trip(port, "PUT", object, &signed, payload).await;
    assert_eq!(put.status, 200);
    assert_eq!(put.header("etag"), Some("\"mock\""));

    let mut create_only = signed.clone();
    create_only.push(("if-none-match", "*"));
    let conflict = round_trip(port, "PUT", object, &create_only, payload).await;
    assert_eq!(conflict.status, 412);
    assert!(
        String::from_utf8_lossy(&conflict.body).contains("PreconditionFailed"),
        "conditional create reports the conflict: {:?}",
        conflict.body
    );

    let get = round_trip(port, "GET", object, &signed, &[]).await;
    assert_eq!(get.status, 200);
    assert_eq!(get.body, payload);
    assert_eq!(get.header("etag"), Some("\"mock\""));
    assert_eq!(get.header("content-type"), Some("application/octet-stream"));
    let length = payload.len().to_string();
    assert_eq!(get.header("content-length"), Some(length.as_str()));

    let head = round_trip(port, "HEAD", object, &signed, &[]).await;
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    assert_eq!(head.header("content-length"), Some(length.as_str()));
    assert_eq!(head.header("etag"), Some("\"mock\""));

    let missing_key = "/kache-test/artifacts/v3/packs/nope.tar.zst";
    let missing_get = round_trip(port, "GET", missing_key, &signed, &[]).await;
    assert_eq!(missing_get.status, 404);
    assert!(
        String::from_utf8_lossy(&missing_get.body).contains("NoSuchKey"),
        "missing key reports NoSuchKey: {:?}",
        missing_get.body
    );

    let missing_head = round_trip(port, "HEAD", missing_key, &signed, &[]).await;
    assert_eq!(missing_head.status, 404);
    assert!(missing_head.body.is_empty());

    let delete = round_trip(port, "DELETE", object, &signed, &[]).await;
    assert_eq!(delete.status, 405);
    assert!(
        String::from_utf8_lossy(&delete.body).contains("MethodNotAllowed"),
        "unsupported method is reported: {:?}",
        delete.body
    );

    assert!(
        store
            .requests()
            .first()
            .is_some_and(|request| request.starts_with("PUT /kache-test/")),
        "request log keeps method and target: {:?}",
        store.requests()
    );
    shutdown_cleanly(shutdown, server).await;
    assert_eq!(store.signed_count(), store.requests().len());
}

#[tokio::test]
async fn list_objects_filters_by_prefix() {
    let store = MockS3::default();
    let (port, shutdown, server) = start_server(store.clone()).await;
    let signed = signed_headers();
    for key in [
        "artifacts/a/first.json",
        "artifacts/a/second.json",
        "artifacts/b/other.json",
    ] {
        let put = round_trip(
            port,
            "PUT",
            &format!("/kache-test/{key}"),
            &signed,
            b"manifest",
        )
        .await;
        assert_eq!(put.status, 200);
    }

    let list = round_trip(
        port,
        "GET",
        "/kache-test?list-type=2&prefix=artifacts%2Fa%2F",
        &signed,
        &[],
    )
    .await;
    assert_eq!(list.status, 200);
    let xml = String::from_utf8_lossy(&list.body).into_owned();
    assert!(
        xml.contains("<KeyCount>2</KeyCount>"),
        "prefix narrows the list: {xml}"
    );
    assert!(
        xml.contains("<Key>artifacts/a/first.json</Key>"),
        "first match is listed: {xml}"
    );
    assert!(
        xml.contains("<Key>artifacts/a/second.json</Key>"),
        "second match is listed: {xml}"
    );
    assert!(
        !xml.contains("artifacts/b/other.json"),
        "outside prefix is excluded: {xml}"
    );

    let unlisted = round_trip(port, "GET", "/kache-test", &signed, &[]).await;
    assert_eq!(unlisted.status, 404);
    assert!(
        String::from_utf8_lossy(&unlisted.body).contains("NoSuchBucket"),
        "a bucket read without list-type is not a list: {:?}",
        unlisted.body
    );

    shutdown_cleanly(shutdown, server).await;
    assert_eq!(store.signed_count(), store.requests().len());
}

#[tokio::test]
async fn rejects_requests_without_sigv4_headers() {
    let store = MockS3::default();
    let (port, shutdown, server) = start_server(store.clone()).await;

    let denied = round_trip(
        port,
        "GET",
        "/kache-test/artifacts/v3/packs/abc.tar.zst",
        &[],
        &[],
    )
    .await;
    assert_eq!(denied.status, 403);
    assert!(
        String::from_utf8_lossy(&denied.body).contains("SignatureDoesNotMatch"),
        "unsigned request names the failure: {:?}",
        denied.body
    );

    let mut partial = signed_headers();
    partial.pop();
    let incomplete = round_trip(
        port,
        "GET",
        "/kache-test/artifacts/v3/packs/abc.tar.zst",
        &partial,
        &[],
    )
    .await;
    assert_eq!(incomplete.status, 403);

    assert_eq!(store.requests().len(), 2);
    assert_eq!(store.signed_count(), 0);
    shutdown_cleanly(shutdown, server).await;
}

#[tokio::test]
async fn rejects_objects_outside_the_bucket() {
    let store = MockS3::default();
    let (port, shutdown, server) = start_server(store.clone()).await;
    let signed = signed_headers();

    let denied = round_trip(port, "GET", "/other-bucket/key", &signed, &[]).await;
    assert_eq!(denied.status, 404);
    assert!(
        String::from_utf8_lossy(&denied.body).contains("NoSuchBucket"),
        "foreign bucket is reported: {:?}",
        denied.body
    );

    shutdown_cleanly(shutdown, server).await;
    assert_eq!(store.signed_count(), store.requests().len());
}

#[tokio::test]
async fn rejects_body_over_limit() {
    let store = MockS3::default();
    let (port, shutdown, server) = start_server(store.clone()).await;
    let signed = signed_headers();

    let oversized = vec![7u8; BODY_LIMIT + 1];
    let rejected = round_trip(
        port,
        "PUT",
        "/kache-test/artifacts/v3/packs/huge.tar.zst",
        &signed,
        &oversized,
    )
    .await;
    assert_eq!(rejected.status, 413);
    assert!(
        String::from_utf8_lossy(&rejected.body).contains("EntityTooLarge"),
        "oversized body is reported: {:?}",
        rejected.body.len()
    );

    shutdown_cleanly(shutdown, server).await;
}
