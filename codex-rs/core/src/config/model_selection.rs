//! Qualified model selectors retain provider-local credentials and model names.
use super::Config;

impl Config {
    /// Selects only a provider already present in the trusted configuration registry.
    pub fn select_model_provider(&mut self, provider_id: &str) -> Result<(), String> {
        if let Some(required) = self.config_layer_stack.required_model_provider()
            && required != provider_id
        {
            return Err(format!("Model provider is required to be '{required}'"));
        }
        let provider = self
            .model_providers
            .get(provider_id)
            .ok_or_else(|| format!("Unknown model provider '{provider_id}'"))?
            .clone();
        self.model_provider_id = provider_id.to_owned();
        self.model_provider = provider;
        Ok(())
    }

    /// `provider::model` selects a provider; unqualified names retain the current one.
    pub fn select_provider_model(&mut self, selector: &str) -> Result<String, String> {
        let model = if let Some((provider, model)) = selector.split_once("::") {
            if provider.is_empty() || model.trim().is_empty() {
                return Err("Use provider::model with a nonempty provider and model".into());
            }
            self.select_model_provider(provider)?;
            model
        } else {
            selector
        };
        if model.trim().is_empty() {
            return Err("Model name cannot be empty".into());
        }
        self.model = Some(model.to_owned());
        Ok(model.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigBuilder;
    #[tokio::test]
    async fn qualified_model_selection_preserves_registry_and_can_switch_back() {
        let home = tempfile::TempDir::new().unwrap();
        let mut config = ConfigBuilder::default()
            .codex_home(home.path().to_owned())
            .fallback_cwd(Some(home.path().to_owned()))
            .build()
            .await
            .unwrap();
        let mut provider = config.model_provider.clone();
        provider.name = "third party".into();
        provider.requires_openai_auth = false;
        provider.base_url = Some("https://api.example.invalid/v1".into());
        config
            .model_providers
            .insert("third_party".into(), provider.clone());
        let registry = config.model_providers.clone();
        assert_eq!(
            config
                .select_provider_model("third_party::vendor/model")
                .unwrap(),
            "vendor/model"
        );
        assert_eq!(config.model_provider, provider);
        assert_eq!(
            config.select_provider_model("another-model").unwrap(),
            "another-model"
        );
        assert_eq!(config.model_provider_id, "third_party");
        assert_eq!(
            config.select_provider_model("openai::gpt-5.4").unwrap(),
            "gpt-5.4"
        );
        assert_eq!(config.model_provider_id, "openai");
        assert_eq!(config.model_providers, registry);
        assert!(config.select_provider_model("unknown::model").is_err());
        assert_eq!(config.model_provider_id, "openai");
        assert!(config.select_provider_model("third_party:: ").is_err());
        assert_eq!(config.model_provider_id, "openai");
    }
}
