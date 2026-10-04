use utoipa::openapi::{Components, Deprecated, OpenApi, RefOr};

pub(super) fn add_route_aliases(openapi: &mut OpenApi) {
    // Historical routes and schema components remain available to existing
    // clients. They share the canonical channel handlers and data model.
    let aliases: Vec<_> = openapi
        .paths
        .paths
        .iter()
        .filter_map(|(path, item)| {
            let alias = if path.starts_with("/v1/channels/") {
                path.replacen("/v1/channels/", "/v1/e/", 1)
            } else if path.starts_with("/v1/agents/") && path.contains("/channels") {
                path.replace("/channels", "/endpoints")
                    .replace("{channel_id}", "{endpoint_id}")
            } else {
                return None;
            };
            let alias = if alias.ends_with("/runtime-auth") {
                alias.replace("{channel_id}", "{endpoint_id}")
            } else {
                alias
            };
            let mut item = item.clone();
            for operation in [
                &mut item.get,
                &mut item.post,
                &mut item.patch,
                &mut item.delete,
            ]
            .into_iter()
            .flatten()
            {
                operation.deprecated = Some(Deprecated::True);
                if let Some(id) = operation.operation_id.as_mut() {
                    if id.contains("channel") {
                        *id = id.replace("channel", "endpoint");
                    } else {
                        id.push_str("_legacy");
                    }
                }
                if alias.contains("{endpoint_id}")
                    && let Some(parameters) = operation.parameters.as_mut()
                {
                    for parameter in parameters {
                        if let RefOr::T(parameter) = parameter
                            && parameter.name == "channel_id"
                        {
                            parameter.name = "endpoint_id".into();
                        }
                    }
                }
            }
            Some((alias, item))
        })
        .collect();
    openapi.paths.paths.extend(aliases);
}

pub(super) fn add_schema_aliases(components: &mut Components) {
    for (canonical, legacy) in [
        ("AgentChannel", "AppChannel"),
        ("ChannelStatus", "EndpointStatus"),
        ("ChannelAuthConfig", "AppEndpointAuthConfig"),
        ("ChannelAuthMode", "AppEndpointAuthMode"),
        ("ChannelAuthProviderConfig", "AppEndpointAuthProviderConfig"),
        ("ChannelAuthRequirements", "AppEndpointAuthRequirements"),
        ("CreateAgentChannelRequest", "CreateAgentEndpointRequest"),
        ("UpdateAgentChannelRequest", "UpdateAgentEndpointRequest"),
        ("TriggerAgentChannelOutput", "TriggerAgentEndpointOutput"),
    ] {
        if let Some(schema) = components.schemas.get(canonical).cloned() {
            components.schemas.insert(legacy.to_string(), schema);
        }
    }
}
