// Curated IDs never become paths or fetch URLs. Previews and saved avatars share one renderer.
use super::avatar::{AvatarError, RenderedAvatarVariant, render_avatar};
use serde::Serialize;
use std::sync::{LazyLock, OnceLock};
use utoipa::ToSchema;

/// Presentation and search metadata for one curated agent avatar.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AvatarPreset {
    /// Stable catalog ID used to select this preset.
    #[schema(example = "familiars-patch")]
    pub id: &'static str,
    /// Display name of the character or design.
    #[schema(example = "Patch")]
    pub name: &'static str,
    /// Visual family used to filter the catalog.
    #[schema(example = "Familiars")]
    pub family: &'static str,
    /// Suggested agent role for this design.
    #[schema(example = "Developer")]
    pub role: &'static str,
    /// Short description of the preset's appearance.
    pub description: &'static str,
    /// Search terms covering the preset's appearance and suggested uses.
    pub keywords: Vec<&'static str>,
}
pub struct PresetAsset {
    pub metadata: AvatarPreset,
    bytes: &'static [u8],
    rendered: OnceLock<Result<Vec<RenderedAvatarVariant>, AvatarError>>,
}
impl PresetAsset {
    pub fn variants(&self) -> Result<&[RenderedAvatarVariant], &AvatarError> {
        self.rendered
            .get_or_init(|| render_avatar(self.bytes, "image/png"))
            .as_deref()
    }
}
macro_rules! preset {
    ($id:literal, $name:literal, $family:literal, $role:literal, $description:literal, [$($keyword:literal),*]) => {
        PresetAsset {
            metadata: AvatarPreset { id: $id, name: $name, family: $family, role: $role, description: $description, keywords: vec![$($keyword),*] },
            bytes: include_bytes!(concat!("../../../assets/avatar-presets/", $id, ".png")),
            rendered: OnceLock::new(),
        }
    };
}
pub static PRESETS: LazyLock<Vec<PresetAsset>> = LazyLock::new(|| {
    vec![
        preset!(
            "watchers-bracket",
            "Bracket",
            "Watchers",
            "Coding",
            "Navy stepped machine face with ivory bracket glasses.",
            [
                "developer",
                "programmer",
                "engineer",
                "software",
                "navy",
                "ivory",
                "gold",
                "robot",
                "machine",
                "geometric",
                "navy",
                "stepped",
                "machine",
                "face",
                "with",
                "ivory",
                "bracket",
                "glasses"
            ]
        ),
        preset!(
            "watchers-relay",
            "Relay",
            "Watchers",
            "Support",
            "Ivory arch face with a gold headset.",
            [
                "help",
                "customer service",
                "headset",
                "navy",
                "ivory",
                "gold",
                "robot",
                "machine",
                "geometric",
                "ivory",
                "arch",
                "face",
                "with",
                "a",
                "gold",
                "headset"
            ]
        ),
        preset!(
            "watchers-beacon",
            "Beacon",
            "Watchers",
            "Marketing",
            "Navy trapezoid face with gold broadcast rays.",
            [
                "promotion",
                "broadcast",
                "communication",
                "navy",
                "ivory",
                "gold",
                "robot",
                "machine",
                "geometric",
                "navy",
                "trapezoid",
                "face",
                "with",
                "gold",
                "broadcast",
                "rays"
            ]
        ),
        preset!(
            "watchers-aperture",
            "Aperture",
            "Watchers",
            "Reviewing",
            "Ivory angular face with a gold optical lens.",
            [
                "reviewer",
                "code review",
                "audit",
                "quality",
                "navy",
                "ivory",
                "gold",
                "robot",
                "machine",
                "geometric",
                "ivory",
                "angular",
                "face",
                "with",
                "a",
                "gold",
                "optical",
                "lens"
            ]
        ),
        preset!(
            "watchers-notch",
            "Notch",
            "Watchers",
            "Bot",
            "Navy stepped face with a warm smile.",
            [
                "assistant",
                "general",
                "automation",
                "navy",
                "ivory",
                "gold",
                "robot",
                "machine",
                "geometric",
                "navy",
                "stepped",
                "face",
                "with",
                "a",
                "warm",
                "smile"
            ]
        ),
        preset!(
            "familiars-patch",
            "Patch",
            "Familiars",
            "Coding",
            "Terracotta fox with navy glasses.",
            [
                "developer",
                "programmer",
                "engineer",
                "software",
                "navy",
                "ivory",
                "gold",
                "animal",
                "angular",
                "geometric",
                "terracotta",
                "fox",
                "with",
                "navy",
                "glasses"
            ]
        ),
        preset!(
            "familiars-buddy",
            "Buddy",
            "Familiars",
            "Support",
            "Friendly navy and ivory dog with a gold headset.",
            [
                "help",
                "customer service",
                "headset",
                "navy",
                "ivory",
                "gold",
                "animal",
                "angular",
                "geometric",
                "friendly",
                "navy",
                "and",
                "ivory",
                "dog",
                "with",
                "a",
                "gold",
                "headset"
            ]
        ),
        preset!(
            "familiars-echo",
            "Echo",
            "Familiars",
            "Marketing",
            "Navy songbird with gold broadcast rays.",
            [
                "promotion",
                "broadcast",
                "communication",
                "navy",
                "ivory",
                "gold",
                "animal",
                "angular",
                "geometric",
                "navy",
                "songbird",
                "with",
                "gold",
                "broadcast",
                "rays"
            ]
        ),
        preset!(
            "familiars-lens",
            "Lens",
            "Familiars",
            "Reviewing",
            "Navy and ivory owl with a gold optical lens.",
            [
                "reviewer",
                "code review",
                "audit",
                "quality",
                "navy",
                "ivory",
                "gold",
                "animal",
                "angular",
                "geometric",
                "navy",
                "and",
                "ivory",
                "owl",
                "with",
                "a",
                "gold",
                "optical",
                "lens"
            ]
        ),
        preset!(
            "familiars-shelly",
            "Shelly",
            "Familiars",
            "Bot",
            "Graphite tortoise with a gold patterned shell.",
            [
                "assistant",
                "general",
                "automation",
                "navy",
                "ivory",
                "gold",
                "animal",
                "angular",
                "geometric",
                "graphite",
                "tortoise",
                "with",
                "a",
                "gold",
                "patterned",
                "shell"
            ]
        ),
        preset!(
            "totems-compose",
            "Compose",
            "Totems",
            "Coding",
            "Ivory brackets holding a gold cube.",
            [
                "developer",
                "programmer",
                "engineer",
                "software",
                "ceramic",
                "sculpture",
                "graphite",
                "ivory",
                "gold",
                "geometric",
                "ivory",
                "brackets",
                "holding",
                "a",
                "gold",
                "cube"
            ]
        ),
        preset!(
            "totems-haven",
            "Haven",
            "Totems",
            "Support",
            "Ceramic arch protecting a gold orb.",
            [
                "help",
                "customer service",
                "headset",
                "ceramic",
                "sculpture",
                "graphite",
                "ivory",
                "gold",
                "geometric",
                "ceramic",
                "arch",
                "protecting",
                "a",
                "gold",
                "orb"
            ]
        ),
        preset!(
            "totems-radiate",
            "Radiate",
            "Totems",
            "Marketing",
            "Ivory and gold sculptural fan.",
            [
                "promotion",
                "broadcast",
                "communication",
                "ceramic",
                "sculpture",
                "graphite",
                "ivory",
                "gold",
                "geometric",
                "ivory",
                "and",
                "gold",
                "sculptural",
                "fan"
            ]
        ),
        preset!(
            "totems-focus",
            "Focus",
            "Totems",
            "Reviewing",
            "Graphite ring framing a gold square.",
            [
                "reviewer",
                "code review",
                "audit",
                "quality",
                "ceramic",
                "sculpture",
                "graphite",
                "ivory",
                "gold",
                "geometric",
                "graphite",
                "ring",
                "framing",
                "a",
                "gold",
                "square"
            ]
        ),
        preset!(
            "totems-balance",
            "Balance",
            "Totems",
            "Bot",
            "Soft ivory and navy ceramic block stack.",
            [
                "assistant",
                "general",
                "automation",
                "ceramic",
                "sculpture",
                "graphite",
                "ivory",
                "gold",
                "geometric",
                "soft",
                "ivory",
                "and",
                "navy",
                "ceramic",
                "block",
                "stack"
            ]
        ),
        preset!(
            "glyphs-syntax",
            "Syntax",
            "Glyphs",
            "Coding",
            "Navy chevrons around a gold square.",
            [
                "developer",
                "programmer",
                "engineer",
                "software",
                "flat",
                "minimal",
                "navy",
                "gold",
                "geometric",
                "navy",
                "chevrons",
                "around",
                "a",
                "gold",
                "square"
            ]
        ),
        preset!(
            "glyphs-connect",
            "Connect",
            "Glyphs",
            "Support",
            "Open navy arcs around a gold circle.",
            [
                "help",
                "customer service",
                "headset",
                "flat",
                "minimal",
                "navy",
                "gold",
                "geometric",
                "open",
                "navy",
                "arcs",
                "around",
                "a",
                "gold",
                "circle"
            ]
        ),
        preset!(
            "glyphs-signal",
            "Signal",
            "Glyphs",
            "Marketing",
            "Three navy and gold broadcast wedges.",
            [
                "promotion",
                "broadcast",
                "communication",
                "flat",
                "minimal",
                "navy",
                "gold",
                "geometric",
                "three",
                "navy",
                "and",
                "gold",
                "broadcast",
                "wedges"
            ]
        ),
        preset!(
            "glyphs-verify",
            "Verify",
            "Glyphs",
            "Reviewing",
            "Navy ring framing a gold check.",
            [
                "reviewer",
                "code review",
                "audit",
                "quality",
                "flat",
                "minimal",
                "navy",
                "gold",
                "geometric",
                "navy",
                "ring",
                "framing",
                "a",
                "gold",
                "check"
            ]
        ),
        preset!(
            "glyphs-orbit",
            "Orbit",
            "Glyphs",
            "Bot",
            "Two navy blocks beside a gold circle.",
            [
                "assistant",
                "general",
                "automation",
                "flat",
                "minimal",
                "navy",
                "gold",
                "geometric",
                "two",
                "navy",
                "blocks",
                "beside",
                "a",
                "gold",
                "circle"
            ]
        ),
        preset!(
            "bloom-mint",
            "Mint",
            "Bloom",
            "Coding",
            "Mint cuboid with purple bracket frames.",
            [
                "developer",
                "programmer",
                "engineer",
                "software",
                "colorful",
                "pastel",
                "soft",
                "cheerful",
                "geometric",
                "mint",
                "cuboid",
                "with",
                "purple",
                "bracket",
                "frames"
            ]
        ),
        preset!(
            "bloom-teal",
            "Teal",
            "Bloom",
            "Support",
            "Teal bean with a peach headset.",
            [
                "help",
                "customer service",
                "headset",
                "colorful",
                "pastel",
                "soft",
                "cheerful",
                "geometric",
                "teal",
                "bean",
                "with",
                "a",
                "peach",
                "headset"
            ]
        ),
        preset!(
            "bloom-coral",
            "Coral",
            "Bloom",
            "Marketing",
            "Coral star with teal broadcast rays.",
            [
                "promotion",
                "broadcast",
                "communication",
                "colorful",
                "pastel",
                "soft",
                "cheerful",
                "geometric",
                "coral",
                "star",
                "with",
                "teal",
                "broadcast",
                "rays"
            ]
        ),
        preset!(
            "bloom-violet",
            "Violet",
            "Bloom",
            "Reviewing",
            "Violet pear with a mint optical lens.",
            [
                "reviewer",
                "code review",
                "audit",
                "quality",
                "colorful",
                "pastel",
                "soft",
                "cheerful",
                "geometric",
                "violet",
                "pear",
                "with",
                "a",
                "mint",
                "optical",
                "lens"
            ]
        ),
        preset!(
            "bloom-lime",
            "Lime",
            "Bloom",
            "Bot",
            "Lime pill with lilac nubs.",
            [
                "assistant",
                "general",
                "automation",
                "colorful",
                "pastel",
                "soft",
                "cheerful",
                "geometric",
                "lime",
                "pill",
                "with",
                "lilac",
                "nubs"
            ]
        ),
    ]
});
pub fn find_preset(id: &str) -> Option<&'static PresetAsset> {
    // THREAT[TM-API-027]: exact allowlist lookup; client IDs are never file paths.
    PRESETS.iter().find(|preset| preset.metadata.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_has_unique_searchable_metadata_and_renderable_assets() {
        let mut ids = std::collections::HashSet::new();
        assert_eq!(PRESETS.len(), 25);
        for preset in PRESETS.iter() {
            let meta = &preset.metadata;
            assert!(ids.insert(meta.id));
            assert!(meta.id.starts_with(&meta.family.to_lowercase()));
            assert!(
                !meta.name.is_empty() && !meta.description.is_empty() && !meta.keywords.is_empty()
            );
            let variants = preset.variants().expect(meta.id);
            assert_eq!(variants.len(), 11);
            for variant in variants {
                let image = image::load_from_memory(&variant.data).unwrap().to_rgba8();
                assert_eq!(image.width(), image.height());
                if variant.variant.starts_with("circle") {
                    assert_eq!(image.get_pixel(0, 0).0[3], 0);
                    assert_eq!(
                        image.get_pixel(image.width() / 2, image.height() / 2).0[3],
                        255
                    );
                }
            }
        }
        assert!(find_preset("../watchers-bracket").is_none());
        assert!(find_preset("unknown").is_none());
    }
}
