use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::future::join_all;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::http_retry::retry_after_header_delay_for_test;
use super::*;

#[derive(Clone)]
struct RestRetryResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
    delay: Duration,
}

async fn rest_retry_test_client(
    responses: Vec<RestRetryResponse>,
) -> (RestClient, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind retry test server");
    let base_url = format!(
        "http://{}",
        listener.local_addr().expect("retry test server address")
    );
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = requests.clone();
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let index = server_requests.fetch_add(1, Ordering::SeqCst);
            let response = responses
                .get(index)
                .cloned()
                .or_else(|| responses.last().cloned())
                .expect("retry test response");
            tokio::spawn(async move {
                let mut request = [0; 4096];
                let _ = socket.read(&mut request).await;
                tokio::time::sleep(response.delay).await;
                let reason = if response.status == 200 {
                    "OK"
                } else if response.status == 429 {
                    "Too Many Requests"
                } else {
                    "Gateway Error"
                };
                let mut wire = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    response.status,
                    reason,
                    response.body.len()
                );
                for (name, value) in response.headers {
                    wire.push_str(&format!("{name}: {value}\r\n"));
                }
                wire.push_str("\r\n");
                wire.push_str(&response.body);
                let _ = socket.write_all(wire.as_bytes()).await;
            });
        }
    });
    let client = RestClient {
        http: reqwest::Client::new(),
        base_url,
        keys: Keys::generate(),
        auth_tag_json: None,
    };
    (client, requests, server)
}

fn retry_response(status: u16, body: &str) -> RestRetryResponse {
    RestRetryResponse {
        status,
        headers: Vec::new(),
        body: body.to_string(),
        delay: Duration::ZERO,
    }
}

#[test]
fn rest_retry_parses_retry_after_seconds_and_http_date() {
    assert_eq!(
        retry_after_header_delay_for_test("3"),
        Some(Duration::from_secs(3))
    );

    let date = (chrono::Utc::now() + chrono::Duration::seconds(3)).to_rfc2822();
    let delay = retry_after_header_delay_for_test(&date).expect("parse HTTP-date Retry-After");
    assert!(delay >= Duration::from_secs(1));
    assert!(delay <= Duration::from_secs(4));
}

#[tokio::test]
async fn rest_retry_query_uses_json_rate_limit_hint() {
    let (client, requests, server) = rest_retry_test_client(vec![
        retry_response(
            429,
            r#"{"error":"rate-limited: quota exceeded; retry in 0s"}"#,
        ),
        retry_response(200, "[]"),
    ])
    .await;

    let result = tokio::time::timeout(Duration::from_secs(3), client.query(&[]))
        .await
        .expect("query retry should be bounded")
        .expect("query should succeed after retry");
    assert_eq!(result, json!([]));
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    server.abort();
}

#[tokio::test]
async fn rest_retry_shares_rate_limit_cooldown_across_queries() {
    let mut limited = retry_response(429, r#"{"error":"quota exceeded"}"#);
    limited.headers.push(("Retry-After".into(), "1".into()));
    let (client, requests, server) = rest_retry_test_client(vec![
        limited,
        retry_response(200, "[]"),
        retry_response(200, "[]"),
    ])
    .await;

    let first = tokio::spawn({
        let client = client.clone();
        async move { client.query(&[]).await }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while requests.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first query must reach the server");
    tokio::time::sleep(Duration::from_millis(50)).await;

    let second = tokio::spawn({
        let client = client.clone();
        async move { client.query(&[nostr::Filter::new().limit(1)]).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        requests.load(Ordering::SeqCst),
        1,
        "a concurrent query must wait while the shared 429 cooldown is active"
    );

    first.await.expect("first query task").expect("first query");
    second
        .await
        .expect("second query task")
        .expect("second query");
    assert_eq!(requests.load(Ordering::SeqCst), 3);
    server.abort();
}

#[tokio::test]
async fn rest_query_coalesces_thirty_overlapping_identical_reads_without_cache() {
    let mut delayed = retry_response(200, "[]");
    delayed.delay = Duration::from_millis(50);
    let (client, requests, server) =
        rest_retry_test_client(vec![delayed, retry_response(200, "[]")]).await;

    let queries = (0..30).map(|_| client.query(&[])).collect::<Vec<_>>();
    let results = tokio::time::timeout(Duration::from_secs(3), join_all(queries))
        .await
        .expect("coalesced queries should be bounded");
    for result in results {
        assert_eq!(result.expect("coalesced query"), json!([]));
    }
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    // A completed flight is removed rather than retained as a result cache.
    assert_eq!(client.query(&[]).await.expect("fresh query"), json!([]));
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    server.abort();
}

#[tokio::test]
async fn rest_query_flight_is_scoped_to_identity_and_filter() {
    let mut delayed = retry_response(200, "[]");
    delayed.delay = Duration::from_millis(50);
    let (client, requests, server) = rest_retry_test_client(vec![
        delayed.clone(),
        retry_response(200, "[]"),
        delayed,
        retry_response(200, "[]"),
    ])
    .await;

    let mut other_identity = client.clone();
    other_identity.keys = Keys::generate();
    let (first, second) = tokio::join!(client.query(&[]), other_identity.query(&[]));
    first.expect("first identity query");
    second.expect("second identity query");
    assert_eq!(requests.load(Ordering::SeqCst), 2);

    let filter = nostr::Filter::new().limit(1);
    let (first, second) = tokio::join!(client.query(&[]), client.query(&[filter]));
    first.expect("empty filter query");
    second.expect("limited filter query");
    assert_eq!(requests.load(Ordering::SeqCst), 4);
    let mut other_auth = client.clone();
    other_auth.auth_tag_json = Some("different-membership-delegation".into());
    let (first, second) = tokio::join!(client.query(&[]), other_auth.query(&[]));
    first.expect("original auth query");
    second.expect("different auth query");
    assert_eq!(requests.load(Ordering::SeqCst), 6);
    server.abort();
}

#[tokio::test]
async fn rest_retry_long_retry_after_returns_bounded_error_without_early_retry() {
    let mut limited = retry_response(429, r#"{"error":"quota exceeded"}"#);
    limited.headers.push(("Retry-After".into(), "120".into()));
    let (client, requests, server) =
        rest_retry_test_client(vec![limited, retry_response(200, "[]")]).await;

    let error = tokio::time::timeout(Duration::from_secs(1), client.query(&[]))
        .await
        .expect("long Retry-After must not block indefinitely")
        .expect_err("client must fail closed rather than retry early");
    assert!(error.to_string().contains("over the client maximum"));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    let error = tokio::time::timeout(Duration::from_secs(1), client.query(&[]))
        .await
        .expect("a new call must fail promptly during a long cooldown")
        .expect_err("a new call must not bypass the server's cooldown");
    assert!(error.to_string().contains("cooldown exceeds"));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn rest_query_cancellation_releases_followers() {
    let mut delayed = retry_response(200, "[]");
    delayed.delay = Duration::from_secs(1);
    let (client, requests, server) =
        rest_retry_test_client(vec![delayed, retry_response(200, "[]")]).await;

    let leader = tokio::spawn({
        let client = client.clone();
        async move { client.query(&[]).await }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while requests.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("leader must reach the server");
    let follower = tokio::spawn({
        let client = client.clone();
        async move { client.query(&[]).await }
    });
    tokio::task::yield_now().await;
    leader.abort();
    let result = tokio::time::timeout(Duration::from_secs(3), follower)
        .await
        .expect("follower must wake after leader cancellation")
        .expect("follower task must finish")
        .expect("replacement leader query must succeed");
    assert_eq!(result, json!([]));
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    server.abort();
}
