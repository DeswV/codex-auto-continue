//! Subscription credentials and provider API keys coexist within one thread.
use anyhow::Result;
use app_test_support::{
    ChatGptAuthFixture, MockResponsesConfig, TestAppServer, write_chatgpt_auth,
};
use codex_app_server_protocol::{
    ClientRequest, ThreadStartParams, TurnCompletedNotification, TurnStartParams,
    TurnStartResponse, TurnStatus, UserInput,
};
use codex_config::types::AuthCredentialsStoreMode;
use codex_features::Feature;
use core_test_support::responses;
use tempfile::TempDir;
use tokio::time::{Duration, timeout};
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, ResponseTemplate};

async fn turn(app: &mut TestAppServer, id: &str, model: &str, text: &str) -> Result<()> {
    let _: TurnStartResponse = app
        .request(|request_id| ClientRequest::TurnStart {
            request_id,
            params: TurnStartParams {
                thread_id: id.into(),
                model: Some(model.into()),
                input: vec![UserInput::Text {
                    text: text.into(),
                    text_elements: vec![],
                }],
                ..Default::default()
            },
        })
        .await?;
    timeout(Duration::from_secs(20), async {
        loop {
            let completed = app
                .read_stream_until_notification_message("turn/completed")
                .await?;
            let completed: TurnCompletedNotification =
                serde_json::from_value(completed.params.unwrap())?;
            if completed.thread_id == id {
                assert_eq!(completed.turn.status, TurnStatus::Completed);
                break;
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_switch_subscription_to_api_and_back_preserves_history_and_auth() -> Result<()> {
    let official = responses::start_mock_server().await;
    let custom = responses::start_mock_server().await;
    app_test_support::mount_workspace_routing(&official).await;
    for (server, text) in [
        (&official, "subscription reply"),
        (&custom, "third-party reply"),
    ] {
        Mock::given(method("POST"))
            .and(path_regex(".*/responses$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(responses::sse(vec![
                        responses::ev_response_created("done"),
                        responses::ev_assistant_message("msg", text),
                        responses::ev_completed("done"),
                    ])),
            )
            .mount(server)
            .await;
    }
    let home = TempDir::new()?;
    write_chatgpt_auth(
        home.path(),
        ChatGptAuthFixture::new("subscription-test-token")
            .account_id("account-123")
            .chatgpt_user_id("user-123")
            .plan_type("pro"),
        AuthCredentialsStoreMode::File,
    )?;
    MockResponsesConfig::new(&official.uri())
        .with_model_provider("openai")
        .with_provider_name("OpenAI")
        .with_model("gpt-5.4")
        .disable_feature(Feature::EnableRequestCompression)
        .with_root_config(&format!(
            "chatgpt_base_url = \"{}/backend-api\"",
            official.uri()
        ))
        .with_provider_config("supports_websockets = false\nrequires_openai_auth = true")
        .with_extra_config(&format!(
            r#"
[model_providers.third_party]
name = "Third party"
base_url = "{}/v1"
wire_api = "responses"
experimental_bearer_token = "third-party-test-key"
requires_openai_auth = false
supports_websockets = false
"#,
            custom.uri()
        ))
        .write(home.path())?;
    let mut app = TestAppServer::builder()
        .with_codex_home(home.path())
        .without_managed_config()
        .with_env_overrides(&[("OPENAI_API_KEY", None)])
        .build_initialized()
        .await?;
    let id = app
        .start_thread(ThreadStartParams::default())
        .await?
        .thread
        .id;
    turn(
        &mut app,
        &id,
        "openai::gpt-5.4",
        "first subscription message",
    )
    .await?;
    turn(
        &mut app,
        &id,
        "third_party::vendor/custom-model",
        "third-party message",
    )
    .await?;
    turn(&mut app, &id, "openai::gpt-5.4", "back on subscription").await?;
    let official_requests = official
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method == "POST" && r.url.path().ends_with("/responses"))
        .collect::<Vec<_>>();
    let custom_requests = custom
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method == "POST" && r.url.path().ends_with("/responses"))
        .collect::<Vec<_>>();
    assert_eq!(official_requests.len(), 2);
    assert_eq!(custom_requests.len(), 1);
    for request in &official_requests {
        assert_eq!(
            request.headers["authorization"].to_str()?,
            "Bearer subscription-test-token"
        );
        assert_eq!(
            request.body_json::<serde_json::Value>()?["model"],
            "gpt-5.4"
        );
    }
    let request = &custom_requests[0];
    assert_eq!(
        request.headers["authorization"].to_str()?,
        "Bearer third-party-test-key"
    );
    assert!(!request.headers.contains_key("chatgpt-account-id"));
    assert_eq!(
        request.body_json::<serde_json::Value>()?["model"],
        "vendor/custom-model"
    );
    let custom_body = request.body_json::<serde_json::Value>()?.to_string();
    assert!(custom_body.contains("first subscription message"));
    assert!(custom_body.contains("subscription reply"));
    let final_body = official_requests[1]
        .body_json::<serde_json::Value>()?
        .to_string();
    assert!(final_body.contains("third-party message"));
    assert!(final_body.contains("third-party reply"));
    assert_eq!(
        std::fs::read_to_string(home.path().join("auth.json"))?.contains("subscription-test-token"),
        true
    );
    Ok(())
}

#[test_case::test_case(false; "subscription_parent_api_child")]
#[test_case::test_case(true; "api_parent_subscription_child")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_switch_parent_and_child_use_independent_providers(reverse: bool) -> Result<()> {
    let official = responses::start_mock_server().await;
    let custom = responses::start_mock_server().await;
    app_test_support::mount_workspace_routing(&official).await;
    for server in [&official, &custom] {
        Mock::given(method("POST"))
            .and(path_regex(".*/responses$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(responses::sse(vec![
                        responses::ev_response_created("finished"),
                        responses::ev_assistant_message("msg", "Finished"),
                        responses::ev_completed("finished"),
                    ])),
            )
            .with_priority(2)
            .mount(server)
            .await;
    }
    let (parent_server, child_server, parent_model, child_model) = if reverse {
        (
            &custom,
            &official,
            "third_party::gpt-5.4",
            "openai::gpt-5.4",
        )
    } else {
        (
            &official,
            &custom,
            "openai::gpt-5.4",
            "third_party::gpt-5.4",
        )
    };
    Mock::given(method("POST")).and(path_regex(".*/responses$"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
            .set_body_string(responses::sse(vec![responses::ev_response_created("spawn"),
                responses::ev_function_call_with_namespace("spawn-worker", "collaboration", "spawn_agent", &serde_json::json!({
                    "task_name":"provider_worker", "message":"Report completion", "fork_turns":"none", "model":child_model
                }).to_string()), responses::ev_completed("spawn")])))
        .up_to_n_times(1).with_priority(1).mount(parent_server).await;
    let home = TempDir::new()?;
    write_chatgpt_auth(
        home.path(),
        ChatGptAuthFixture::new("subscription-test-token")
            .account_id("account-123")
            .chatgpt_user_id("user-123")
            .plan_type("plus"),
        AuthCredentialsStoreMode::File,
    )?;
    MockResponsesConfig::new(&official.uri())
        .with_model_provider("openai")
        .with_provider_name("OpenAI")
        .with_model("gpt-5.4")
        .disable_feature(Feature::EnableRequestCompression)
        .enable_feature(Feature::MultiAgentV2)
        .with_root_config(&format!(
            "chatgpt_base_url = \"{}/backend-api\"",
            official.uri()
        ))
        .with_provider_config("supports_websockets = false\nrequires_openai_auth = true")
        .with_extra_config(&format!(
            r#"
[model_providers.third_party]
name = "Third party"
base_url = "{}/v1"
wire_api = "responses"
experimental_bearer_token = "third-party-test-key"
requires_openai_auth = false
supports_websockets = false
[multi_agent_v2]
expose_spawn_agent_model_overrides = true
"#,
            custom.uri()
        ))
        .write(home.path())?;
    let mut app = TestAppServer::builder()
        .with_codex_home(home.path())
        .without_managed_config()
        .with_env_overrides(&[("OPENAI_API_KEY", None)])
        .build_initialized()
        .await?;
    let id = app
        .start_thread(ThreadStartParams::default())
        .await?
        .thread
        .id;
    turn(&mut app, &id, parent_model, "Spawn the worker now").await?;
    timeout(Duration::from_secs(20), async {
        loop {
            if child_server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.method == "POST" && r.url.path().ends_with("/responses"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    let requests = child_server.received_requests().await.unwrap();
    let child = requests
        .iter()
        .find(|r| r.method == "POST" && r.url.path().ends_with("/responses"))
        .unwrap();
    assert_eq!(
        child.headers["authorization"].to_str()?,
        if reverse {
            "Bearer subscription-test-token"
        } else {
            "Bearer third-party-test-key"
        }
    );
    assert_eq!(child.body_json::<serde_json::Value>()?["model"], "gpt-5.4");
    if !reverse {
        assert!(!child.headers.contains_key("chatgpt-account-id"));
    }
    Ok(())
}
