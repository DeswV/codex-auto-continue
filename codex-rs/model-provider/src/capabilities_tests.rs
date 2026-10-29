use super::*;
use crate::create_model_provider;
use codex_model_provider_info::ModelProviderCapabilities;
use pretty_assertions::assert_eq;

#[test]
fn capability_overrides_preserve_unspecified_provider_defaults() {
    for info in [
        ModelProviderInfo::default(),
        ModelProviderInfo::create_openai_provider(/*base_url*/ None),
        ModelProviderInfo {
            name: "Azure".to_string(),
            ..Default::default()
        },
    ] {
        let defaults = create_model_provider(info.clone(), /*auth_manager*/ None).capabilities();
        let provider = create_model_provider(
            ModelProviderInfo {
                capabilities: Some(ModelProviderCapabilities {
                    external_web_access: Some(false),
                    ..Default::default()
                }),
                ..info
            },
            /*auth_manager*/ None,
        );
        assert_eq!(
            provider.capabilities(),
            ProviderCapabilities {
                external_web_access: false,
                ..defaults
            },
        );
    }
}

#[test]
fn third_party_provider_does_not_receive_subscription_attestation() {
    let auth = codex_login::AuthManager::from_auth_for_testing(
        codex_login::CodexAuth::create_dummy_chatgpt_auth_for_testing(),
    );
    let official = ModelProviderInfo::create_openai_provider(None);
    assert!(create_model_provider(official.clone(), Some(auth.clone())).supports_attestation());
    let custom = ModelProviderInfo {
        name: "Third party".into(),
        requires_openai_auth: false,
        base_url: Some("https://api.example.invalid/v1".into()),
        ..official
    };
    assert!(!create_model_provider(custom, Some(auth)).supports_attestation());
}
