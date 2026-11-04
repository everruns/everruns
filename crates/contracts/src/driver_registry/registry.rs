use super::*;

/// Registry for LLM drivers
///
/// Enables dependency inversion: vendor modules in everruns-drivers
/// register their drivers at startup. The core has no direct knowledge of implementations.
///
/// # Example
///
/// ```ignore
/// use everruns_contracts::{DriverRegistry, DriverId};
/// use everruns_drivers::anthropic::register_driver;
/// use everruns_drivers::openai::register_driver as register_openai;
///
/// let mut registry = DriverRegistry::new();
/// everruns_drivers::anthropic::register_driver(&mut registry);
/// everruns_drivers::openai::register_driver(&mut registry);
///
/// // Later, create a driver from config
/// let driver = registry.create_chat_driver(&config)?;
/// ```
#[derive(Clone, Default)]
pub struct DriverRegistry {
    descriptors: HashMap<DriverId, DriverDescriptor>,
    providers: crate::runtime_provider::RuntimeProviderRegistry,
}

impl DriverRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            descriptors: HashMap::new(),
            providers: crate::runtime_provider::RuntimeProviderRegistry::new(),
        }
    }

    /// Register an application-supplied runtime provider directly.
    pub fn register_provider(
        &mut self,
        provider: crate::runtime_provider::RuntimeProvider,
    ) -> Result<()> {
        self.providers.register(provider)
    }

    /// Explicitly replace an application-supplied runtime provider.
    pub fn replace_provider(
        &mut self,
        provider: crate::runtime_provider::RuntimeProvider,
    ) -> Option<Arc<crate::runtime_provider::RuntimeProvider>> {
        self.providers.replace(provider)
    }

    /// Look up a directly registered runtime provider by service identity.
    pub fn provider(
        &self,
        id: &crate::runtime_provider::ProviderKey,
    ) -> Option<Arc<crate::runtime_provider::RuntimeProvider>> {
        self.providers.get(id)
    }

    /// Register a full driver descriptor.
    ///
    /// Panics if a descriptor is already registered for the same driver id —
    /// silent overwrites hide double-registration bugs. Use
    /// [`Self::register_descriptor_or_replace`] to overwrite intentionally.
    pub fn register_descriptor(&mut self, descriptor: DriverDescriptor) {
        if self.descriptors.contains_key(&descriptor.id) {
            panic!(
                "driver already registered for provider '{}'; \
                 use register_descriptor_or_replace to overwrite intentionally",
                descriptor.id
            );
        }
        self.descriptors.insert(descriptor.id.clone(), descriptor);
    }

    /// Register a full driver descriptor, replacing any existing one.
    pub fn register_descriptor_or_replace(&mut self, descriptor: DriverDescriptor) {
        self.descriptors.insert(descriptor.id.clone(), descriptor);
    }

    /// Register a driver factory for a provider type.
    ///
    /// Panics if a factory is already registered for `provider_type` — silent
    /// overwrites hide double-registration bugs. Use
    /// [`Self::register_or_replace`] to overwrite intentionally.
    pub fn register<F>(&mut self, provider_type: impl Into<DriverId>, factory: F)
    where
        F: Fn(&DriverConfig) -> BoxedChatDriver + Send + Sync + 'static,
    {
        self.register_descriptor(DriverDescriptor::chat_only(provider_type, factory));
    }

    /// Register a driver factory, replacing any existing one for the provider.
    ///
    /// Use when overwriting is intentional (e.g. swapping in an `LlmSim` driver
    /// for tests). Prefer [`Self::register`] otherwise so duplicates surface.
    pub fn register_or_replace<F>(&mut self, provider_type: impl Into<DriverId>, factory: F)
    where
        F: Fn(&DriverConfig) -> BoxedChatDriver + Send + Sync + 'static,
    {
        self.register_descriptor_or_replace(DriverDescriptor::chat_only(provider_type, factory));
    }

    /// Register a driver factory for an embedder-defined external provider,
    /// keyed by its canonical id. The id is normalized to lowercase (via
    /// [`DriverId::external`]) so it matches parsed lookups regardless of
    /// the casing stored in the database or sent on the wire.
    pub fn register_external<F>(&mut self, id: impl AsRef<str>, factory: F)
    where
        F: Fn(&DriverConfig) -> BoxedChatDriver + Send + Sync + 'static,
    {
        let mut descriptor = DriverDescriptor::chat_only(DriverId::external(id), factory);
        descriptor.credential_schema = CredentialFormSchema::empty();
        self.register_descriptor(descriptor);
    }

    /// Create an LLM driver based on configuration
    ///
    /// This function does not fall back to environment variables. Keys should
    /// be decrypted by the host and passed here. A selected provider with
    /// missing credentials still produces a driver so commands can inspect the
    /// turn and repair configuration; that driver rejects every provider
    /// operation locally before network I/O.
    ///
    /// Returns `DriverNotRegistered` error if no driver is registered for the provider type.
    pub fn create_chat_driver(&self, config: &ProviderConfig) -> Result<BoxedChatDriver> {
        if let Some(provider) = self.providers.get(&config.provider) {
            return Ok(RequestOptionsDriver::wrap(
                (*provider).clone().into_boxed_driver(),
                &config.request_options,
            ));
        }
        let descriptor = self.descriptors.get(&config.provider_type).ok_or_else(|| {
            AgentLoopError::driver_not_registered(config.provider_type.to_string())
        })?;
        descriptor.validate_api(config.api.as_deref())?;
        // Look up the descriptor and its chat factory for this provider type
        let factory = descriptor.chat.as_ref().ok_or_else(|| {
            AgentLoopError::llm(format!(
                "Provider driver '{}' does not implement the chat service.",
                config.provider_type
            ))
        })?;

        // Create the driver using the factory
        let driver_config = DriverConfig::from_provider_config(config);
        let driver = if let Some(provider) = &descriptor.provider {
            provider(&driver_config).into_boxed_driver()
        } else {
            factory(&driver_config)
        };
        let mut credential_fields = driver_config.credentials.clone();
        if let Some(serde_json::Value::Object(extra)) = &driver_config.metadata.extra {
            for (name, value) in extra {
                if let Some(value) = value.as_str() {
                    credential_fields
                        .entry(name.clone())
                        .or_insert_with(|| value.to_string());
                }
            }
        }
        let credential_errors = descriptor.credential_schema.validate(&credential_fields);
        if credential_errors.is_empty() {
            Ok(RequestOptionsDriver::wrap(driver, &config.request_options))
        } else {
            let message = if descriptor.credential_schema.fields.len() == 1
                && descriptor.credential_schema.fields[0].name == "api_key"
            {
                "API key is required. Configure the API key in provider settings.".to_string()
            } else {
                format!(
                    "Provider credentials are required. Configure provider settings: {}",
                    credential_errors.join(" ")
                )
            };
            Ok(Box::new(CredentialGateDriver {
                inner: driver,
                message,
            }))
        }
    }

    /// Check if a driver is registered for a provider type
    pub fn has_driver(&self, provider_type: &DriverId) -> bool {
        self.descriptors.contains_key(provider_type)
    }

    /// Get the registered descriptor for a provider type.
    pub fn descriptor(&self, provider_type: &DriverId) -> Option<&DriverDescriptor> {
        self.descriptors.get(provider_type)
    }

    /// Curated model recommendations for a provider, if its driver declares any.
    /// External/custom drivers report `None`; callers should fall back to live
    /// discovery (`rank_discovered_models`).
    pub fn recommended_models(
        &self,
        provider_type: &DriverId,
    ) -> Option<ProviderRecommendedModels> {
        self.descriptors.get(provider_type)?.recommended_models
    }

    /// Whether the registered driver declares the given service.
    pub fn supports(&self, provider_type: &DriverId, service: ServiceKind) -> bool {
        self.descriptors
            .get(provider_type)
            .is_some_and(|d| d.supports(service))
    }

    /// Driver ids whose descriptors declare the given service.
    pub fn providers_for(&self, service: ServiceKind) -> Vec<DriverId> {
        self.descriptors
            .values()
            .filter(|d| d.supports(service))
            .map(|d| d.id.clone())
            .collect()
    }

    /// Get the list of registered provider types
    pub fn registered_providers(&self) -> Vec<DriverId> {
        self.descriptors.keys().cloned().collect()
    }

    /// Runtime provider ids registered directly by an application.
    pub fn registered_provider_ids(&self) -> Vec<String> {
        self.providers.ids()
    }

    /// Compose the selected account with its typed model services.
    pub fn create_provider(&self, config: &ProviderConfig) -> Result<crate::Provider> {
        if let Some(provider) = self.providers.get(&config.provider) {
            return Ok((*provider).clone());
        }
        let descriptor = self.descriptor(&config.provider_type).ok_or_else(|| {
            AgentLoopError::driver_not_registered(config.provider_type.to_string())
        })?;
        descriptor.validate_api(config.api.as_deref())?;
        let headers = config.request_options.headers.clone();
        let config = DriverConfig::from_provider_config(config);
        let errors = descriptor.credential_schema.validate(&config.credentials);
        if !errors.is_empty() {
            return Err(AgentLoopError::Configuration(
                "Provider credentials are required. Configure provider settings.".into(),
            ));
        }
        if let Some(factory) = &descriptor.provider {
            let mut provider = factory(&config);
            for header in &headers {
                provider = provider.header(&header.name, &header.value);
            }
            return Ok(provider);
        }
        let factory = descriptor
            .chat
            .as_ref()
            .ok_or_else(|| AgentLoopError::Configuration("Provider has no model service".into()))?;
        Ok(
            crate::Provider::from_driver(config.provider.clone(), factory(&config).into())
                .with_driver_id(config.provider_type.clone()),
        )
    }

    /// Create the realtime (speech-to-speech) driver of a provider, with the
    /// endpoint its calls go to. Fails when the provider has no realtime
    /// service.
    pub fn create_realtime_driver(
        &self,
        config: &ProviderConfig,
    ) -> Result<(
        crate::voice::SharedRealtimeDriver,
        crate::runtime_provider::ProviderEndpoint,
    )> {
        self.create_provider(config)?.realtime()
    }

    /// Create an embeddings driver based on configuration.
    ///
    /// API keys must be provided in the config for real providers. Exception:
    /// `LlmSim` and `External` providers do not require an API key.
    ///
    /// Returns an error if the driver is not registered or does not implement
    /// the embeddings service.
    pub fn create_embeddings_driver(
        &self,
        config: &ProviderConfig,
    ) -> std::result::Result<BoxedEmbeddingsDriver, EmbeddingsDriverError> {
        if let Some(provider) = self.providers.get(&config.provider) {
            return (*provider).clone().into_embeddings_driver();
        }
        if self
            .descriptor(&config.provider_type)
            .is_some_and(|d| d.provider.is_some())
        {
            return self
                .create_provider(config)
                .map_err(|e| EmbeddingsDriverError::Provider(e.to_string()))?
                .into_embeddings_driver();
        }
        let requires_api_key = config.provider_type != DriverId::LlmSim;
        if requires_api_key && config.api_key.is_none() {
            return Err(EmbeddingsDriverError::Provider(
                "API key is required. Configure the API key in provider settings.".to_string(),
            ));
        }
        let descriptor = self.descriptors.get(&config.provider_type).ok_or_else(|| {
            EmbeddingsDriverError::Provider(format!(
                "No driver registered for provider '{}'",
                config.provider_type
            ))
        })?;
        let factory = descriptor.embeddings.as_ref().ok_or_else(|| {
            EmbeddingsDriverError::Provider(format!(
                "Provider driver '{}' does not implement the embeddings service.",
                config.provider_type
            ))
        })?;
        let driver_config = DriverConfig::from_provider_config(config);
        Ok(factory(&driver_config))
    }
}
