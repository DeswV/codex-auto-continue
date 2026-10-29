//! Real websocket clients disconnect/reconnect while the server owns recovery.
use super::connection_handling_websocket::*;
use anyhow::{Context, Result};
use app_test_support::MockResponsesConfig;
use codex_app_server_protocol::AutoRetryNotification;
use core_test_support::responses;
use serde_json::json;
use tempfile::TempDir;
use tokio::time::{Duration, timeout};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn fixture(
    message: String,
    goal_enabled: bool,
) -> Result<(
    TempDir,
    MockServer,
    tokio::process::Child,
    std::net::SocketAddr,
)> {
    let server = MockServer::start().await;
    let failure = responses::sse(vec![
        responses::ev_response_created("failed"),
        json!({
            "type": "response.failed", "response": { "id": "failed", "error": {"code":"invalid_prompt", "message": message} }
        }),
    ]);
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(failure)
                .set_delay(Duration::from_secs(2)),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(responses::sse(vec![
                    responses::ev_response_created("recovered"),
                    responses::ev_completed("recovered"),
                ])),
        )
        .with_priority(2)
        .mount(&server)
        .await;
    let home = TempDir::new()?;
    let mut config = MockResponsesConfig::new(&server.uri())
        .with_root_config("thread_unload_delay_secs = 1")
        .with_provider_config("supports_websockets = false");
    if goal_enabled {
        config = config
            .enable_feature(codex_features::Feature::Goals)
            .with_model("gpt-5.4");
    }
    config.write(home.path())?;
    let (child, address) = spawn_websocket_server(home.path()).await?;
    Ok((home, server, child, address))
}
async fn start(client: &mut WsClient) -> Result<String> {
    send_initialize_request(client, 1, "auto-retry-test").await?;
    read_response_for_id(client, 1).await?;
    send_request(client, "thread/start", 2, Some(json!({}))).await?;
    let response = read_response_for_id(client, 2).await?;
    let id = response.result["thread"]["id"]
        .as_str()
        .context("thread id")?
        .to_owned();
    send_request(
        client,
        "turn/start",
        3,
        Some(json!({"threadId": id, "input": [{"type":"text", "text":"Hello"}]})),
    )
    .await?;
    read_response_for_id(client, 3).await?;
    Ok(id)
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_retry_continues_with_no_connected_clients() -> Result<()> {
    let (_home, server, mut child, address) =
        fixture("model capability error".into(), false).await?;
    let mut client = connect_websocket(address).await?;
    start(&mut client).await?;
    client.close(None).await?;
    drop(client);
    timeout(Duration::from_secs(30), async {
        loop {
            if server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.url.path() == "/v1/responses")
                .count()
                >= 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("server did not retry without a TUI")?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path() == "/v1/responses")
            .count(),
        2
    );
    child.kill().await?;
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_retry_reconnect_shows_original_deadline_and_can_cancel() -> Result<()> {
    let reset = (chrono::Local::now() + chrono::Duration::minutes(5))
        .format("%b %e, %Y %I:%M %p")
        .to_string();
    let (_home, _server, mut child, address) =
        fixture(format!("usage limit reached. try again at {reset}."), false).await?;
    let mut client = connect_websocket(address).await?;
    let id = start(&mut client).await?;
    let first = timeout(
        DEFAULT_READ_TIMEOUT,
        read_notification_for_method(&mut client, "thread/autoRetry"),
    )
    .await??;
    let first: AutoRetryNotification =
        serde_json::from_value(first.params.context("retry status")?)?;
    client.close(None).await?;
    drop(client);
    tokio::time::sleep(Duration::from_millis(2100)).await;
    let mut reconnected = connect_websocket(address).await?;
    send_initialize_request(&mut reconnected, 1, "reconnected-tui").await?;
    read_response_for_id(&mut reconnected, 1).await?;
    send_request(
        &mut reconnected,
        "thread/resume",
        2,
        Some(json!({"threadId": id})),
    )
    .await?;
    read_response_for_id(&mut reconnected, 2).await?;
    let status = timeout(
        DEFAULT_READ_TIMEOUT,
        read_notification_for_method(&mut reconnected, "thread/autoRetry"),
    )
    .await??;
    let status: AutoRetryNotification =
        serde_json::from_value(status.params.context("reconnected status")?)?;
    assert_eq!(status.retry_at, first.retry_at);
    assert_eq!(status.turn_id, first.turn_id);
    send_request(
        &mut reconnected,
        "turn/interrupt",
        3,
        Some(json!({"threadId": id, "turnId": status.turn_id})),
    )
    .await?;
    // A cancelled wait must not be replayed as active to a newly attached client.
    let cancelled = timeout(
        DEFAULT_READ_TIMEOUT,
        read_notification_for_method(&mut reconnected, "thread/autoRetry"),
    )
    .await??;
    let cancelled: AutoRetryNotification =
        serde_json::from_value(cancelled.params.context("cancelled status")?)?;
    assert_eq!(cancelled.retry_at, None);
    read_response_for_id(&mut reconnected, 3).await?;
    child.kill().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_retry_resumes_blocked_goal_without_clients() -> Result<()> {
    let (_home, server, mut child, address) =
        fixture("model capability error".into(), true).await?;
    let mut client = connect_websocket(address).await?;
    let id = start(&mut client).await?;
    send_request(&mut client, "thread/goal/set", 4, Some(json!({
        "threadId": id, "objective": "Finish the recovery test", "status": "active", "tokenBudget": 10000
    }))).await?;
    read_response_for_id(&mut client, 4).await?;
    client.close(None).await?;
    drop(client);
    timeout(Duration::from_secs(30), async {
        loop {
            if server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.url.path() == "/v1/responses")
                .count()
                >= 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("server did not resume the blocked goal without a TUI")?;
    child.kill().await?;
    Ok(())
}
