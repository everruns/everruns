// Agent avatar -> Slack app icon.
//
// Each Slack endpoint is its own Slack app, and what people see next to the
// agent's messages is that app's icon. A manifest cannot carry an icon, so the
// avatar is pushed with `apps.icon.set` through the org's app configuration
// token: when the app is created, again after OAuth (Slack may ignore an icon
// set before the bot user exists), and whenever the avatar changes.
//
// When the API origin is public HTTPS, Slack fetches the immutable avatar URL.
// That is the same JSON call as the manifest methods. A local or private origin
// cannot be fetched, so the PNG is uploaded instead. `apps.icon.set` is about
// one call a minute and can answer `invalid_app` in the moment after create;
// both are retried. A failure is logged rather than failing an upload or an
// install. Endpoints set up with the manual manifest flow have no configuration
// token; there the operator uploads the icon in Slack (the UI offers the 512px
// PNG for that). Removing an avatar leaves the last icon in place: Slack has
// no "reset".

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use super::avatar::{AvatarShape, SLACK_ICON_SIZE, variant_name};
use crate::records::slack_provisioning::{SlackAppProvisioner, SlackProvisioningError};
use crate::storage::{EncryptionService, StorageBackend};

/// Slack's manifest quota, and `apps.icon.set`, reset about once a minute.
/// Tests shorten the wait so a retry does not hold the suite.
#[cfg(not(test))]
const RATE_LIMIT_RETRY_DELAY: Duration = Duration::from_secs(60);
#[cfg(test)]
const RATE_LIMIT_RETRY_DELAY: Duration = Duration::from_millis(500);

#[cfg(not(test))]
const TRANSIENT_RETRY_DELAY: Duration = Duration::from_secs(2);
#[cfg(test)]
const TRANSIENT_RETRY_DELAY: Duration = Duration::from_millis(50);

const ICON_ATTEMPTS: u32 = 3;

/// Background the generated Slack manifest uses. Transparent pixels are
/// flattened onto it before a file upload; Slack rejects some alpha PNGs.
const SLACK_ICON_BACKGROUND: [u8; 3] = [0x1a, 0x1a, 0x2e];

/// The PNG to use as a Slack app icon for an avatar.
pub async fn slack_icon_png(db: &StorageBackend, avatar_id: Uuid) -> Option<Vec<u8>> {
    match db
        .get_agent_avatar_variant(
            avatar_id,
            &variant_name(AvatarShape::Square, SLACK_ICON_SIZE),
        )
        .await
    {
        Ok(row) => row.map(|row| row.data),
        Err(error) => {
            tracing::warn!(%avatar_id, %error, "Could not load avatar for Slack icon");
            None
        }
    }
}

/// Set one Slack app's icon from the agent's current avatar, if it has one.
pub async fn push_agent_avatar_to_slack_app(
    db: &StorageBackend,
    provisioner: &dyn SlackAppProvisioner,
    org_id: i64,
    agent_id: Uuid,
    team_id: Option<&str>,
    app_id: &str,
    api_base_url: &str,
) {
    let avatar_id = match db
        .get_agent(
            org_id,
            everruns_contracts::typed_id::AgentId::from_uuid(agent_id),
        )
        .await
    {
        Ok(Some(row)) => row.avatar_id,
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(%agent_id, %error, "Could not load agent for Slack icon");
            None
        }
    };
    let Some(avatar_id) = avatar_id else {
        return;
    };
    set_icon_with_retry(
        provisioner,
        db,
        org_id,
        team_id,
        app_id,
        avatar_id,
        api_base_url,
    )
    .await;
}

/// Set the icon of every provisioned Slack app of an agent to `avatar_id`.
/// Returns how many apps were updated.
pub async fn push_avatar_to_agent_slack_apps(
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    provisioner: Arc<dyn SlackAppProvisioner>,
    org_id: i64,
    agent_id: Uuid,
    avatar_id: Uuid,
    api_base_url: &str,
) -> usize {
    let channels = match db.list_agent_channels(org_id, agent_id).await {
        Ok(channels) => channels,
        Err(error) => {
            tracing::warn!(%agent_id, %error, "Could not list endpoints for Slack icon sync");
            return 0;
        }
    };
    let mut apps = Vec::new();
    for row in channels {
        if row.channel_type != "slack" {
            continue;
        }
        let Ok((_, channel)) =
            crate::api::channel_ingress::row_to_ingress(encryption.as_ref(), row)
        else {
            continue;
        };
        if let Some(app) = channel.slack_config().and_then(|c| c.provisioned_app) {
            apps.push(app);
        }
    }
    let mut updated = 0;
    for app in apps {
        if set_icon_with_retry(
            provisioner.as_ref(),
            db.as_ref(),
            org_id,
            app.team_id.as_deref(),
            &app.app_id,
            avatar_id,
            api_base_url,
        )
        .await
        {
            updated += 1;
        }
    }
    updated
}

/// `true` when Slack accepted the icon.
async fn set_icon_with_retry(
    provisioner: &dyn SlackAppProvisioner,
    db: &StorageBackend,
    org_id: i64,
    team_id: Option<&str>,
    app_id: &str,
    avatar_id: Uuid,
    api_base_url: &str,
) -> bool {
    for attempt in 0..ICON_ATTEMPTS {
        match set_icon(
            provisioner,
            db,
            org_id,
            team_id,
            app_id,
            avatar_id,
            api_base_url,
        )
        .await
        {
            Ok(updated) => return updated,
            Err(error) if attempt + 1 < ICON_ATTEMPTS && is_retryable(&error) => {
                tokio::time::sleep(retry_delay(&error)).await;
            }
            Err(error) => {
                tracing::warn!(app_id, %error, "Could not set Slack app icon from agent avatar");
                return false;
            }
        }
    }
    false
}

/// `Ok(false)` when there is no image to send.
async fn set_icon(
    provisioner: &dyn SlackAppProvisioner,
    db: &StorageBackend,
    org_id: i64,
    team_id: Option<&str>,
    app_id: &str,
    avatar_id: Uuid,
    api_base_url: &str,
) -> Result<bool, SlackProvisioningError> {
    if let Some(url) = public_avatar_icon_url(api_base_url, avatar_id) {
        match provisioner
            .set_app_icon_url(org_id, team_id, app_id, &url)
            .await
        {
            Ok(()) => return Ok(true),
            Err(SlackProvisioningError::Rejected(code)) if url_needs_file_upload(&code) => {
                tracing::warn!(
                    app_id,
                    code,
                    "Slack could not fetch the avatar URL; uploading the PNG"
                );
            }
            Err(SlackProvisioningError::Unavailable) => {}
            Err(error) => return Err(error),
        }
    }
    let Some(png) = slack_icon_png(db, avatar_id).await else {
        return Ok(false);
    };
    provisioner
        .set_app_icon(org_id, team_id, app_id, opaque_slack_png(&png))
        .await?;
    Ok(true)
}

/// Public HTTPS avatar URL Slack can fetch, or `None` when this deployment
/// is local or private and the PNG must be uploaded instead.
fn public_avatar_icon_url(api_base_url: &str, avatar_id: Uuid) -> Option<String> {
    let base = api_base_url.trim().trim_end_matches('/');
    let host = https_host(base)?;
    if !slack_can_fetch(host) {
        return None;
    }
    let path = crate::records::AgentAvatar::from_uuid(avatar_id)
        .path(AvatarShape::Square, SLACK_ICON_SIZE);
    Some(format!("{base}{path}"))
}

fn https_host(base: &str) -> Option<&str> {
    let rest = base.strip_prefix("https://")?;
    let hostport = rest.split(['/', '?', '#']).next().unwrap_or("");
    if hostport.is_empty() {
        return None;
    }
    let hostport = hostport.rsplit('@').next().unwrap_or(hostport);
    let host = if let Some(host) = hostport.strip_prefix('[') {
        host.split(']').next()?
    } else {
        hostport.split(':').next()?
    };
    (!host.is_empty()).then_some(host)
}

fn slack_can_fetch(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    !(host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host == "0.0.0.0"
        || host == "::1"
        || is_private_ipv4(&host))
}

fn is_private_ipv4(host: &str) -> bool {
    let Some((a, rest)) = host.split_once('.') else {
        return false;
    };
    let Ok(a) = a.parse::<u8>() else {
        return false;
    };
    let b = rest
        .split('.')
        .next()
        .and_then(|part| part.parse::<u8>().ok());
    match (a, b) {
        (10 | 127, _) => true,
        (192, Some(168)) => true,
        (172, Some(b)) if (16..=31).contains(&b) => true,
        (169, Some(254)) => true,
        _ => false,
    }
}

fn url_needs_file_upload(code: &str) -> bool {
    matches!(
        code,
        "icon_not_accessible" | "invalid_url" | "error_bad_format"
    )
}

fn is_retryable(error: &SlackProvisioningError) -> bool {
    match error {
        SlackProvisioningError::Rejected(code) => matches!(
            code.as_str(),
            "ratelimited"
                | "invalid_app"
                | "internal_error"
                | "service_unavailable"
                | "request_timeout"
        ),
        SlackProvisioningError::Unreachable(_) => true,
        _ => false,
    }
}

fn retry_delay(error: &SlackProvisioningError) -> Duration {
    match error {
        SlackProvisioningError::Rejected(code) if code == "ratelimited" => RATE_LIMIT_RETRY_DELAY,
        _ => TRANSIENT_RETRY_DELAY,
    }
}

fn opaque_slack_png(png: &[u8]) -> Vec<u8> {
    let Ok(image) = image::load_from_memory(png) else {
        return png.to_vec();
    };
    let rgba = image.to_rgba8();
    let mut flat = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let image::Rgba([r, g, b, a]) = *pixel;
        let alpha = u16::from(a);
        let mix = |channel: u8, bg: u8| -> u8 {
            ((u16::from(channel) * alpha + u16::from(bg) * (255 - alpha)) / 255) as u8
        };
        flat.put_pixel(
            x,
            y,
            image::Rgb([
                mix(r, SLACK_ICON_BACKGROUND[0]),
                mix(g, SLACK_ICON_BACKGROUND[1]),
                mix(b, SLACK_ICON_BACKGROUND[2]),
            ]),
        );
    }
    let mut buffer = std::io::Cursor::new(Vec::new());
    if image::DynamicImage::ImageRgb8(flat)
        .write_to(&mut buffer, image::ImageFormat::Png)
        .is_err()
    {
        return png.to_vec();
    }
    buffer.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_icon_url_uses_the_square_512_variant() {
        let url = public_avatar_icon_url("https://api.example.com/", Uuid::nil()).unwrap();
        assert!(url.starts_with("https://api.example.com/v1/avatars/"));
        assert!(url.ends_with("/square-512.png"));
        let prefixed = public_avatar_icon_url("https://app.example.com/api", Uuid::nil()).unwrap();
        assert!(prefixed.starts_with("https://app.example.com/api/v1/avatars/"));
        assert!(prefixed.ends_with("/square-512.png"));
    }

    #[test]
    fn local_and_private_bases_are_uploaded_instead() {
        for base in [
            "",
            "http://api.example.com",
            "https://localhost:27100",
            "https://127.0.0.1:27100",
            "https://10.0.0.5",
            "https://192.168.1.2",
            "https://172.16.0.4",
            "https://[::1]",
        ] {
            assert!(
                public_avatar_icon_url(base, Uuid::nil()).is_none(),
                "{base}"
            );
        }
    }

    #[test]
    fn transparent_png_is_flattened_onto_the_manifest_background() {
        let mut image = image::RgbaImage::new(1, 1);
        image.put_pixel(0, 0, image::Rgba([255, 0, 0, 0]));
        let mut buffer = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();
        let flat = opaque_slack_png(&buffer.into_inner());
        let decoded = image::load_from_memory(&flat).unwrap().to_rgb8();
        assert_eq!(decoded.get_pixel(0, 0).0, SLACK_ICON_BACKGROUND);
    }
}
