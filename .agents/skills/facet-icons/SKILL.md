---
name: facet-icons
description: Extend or refine Everruns' custom Facet domain icon family while preserving the approved Intent Agent mark and Slate's design grammar.
metadata:
  internal: true
user-invocable: true
---

# Extend Facet icons

Read [Iconography](../../../knowledge/ui/iconography.md) for intent and acceptance, and
[DESIGN.md](../../../apps/ui/DESIGN.md) for the current Slate context. Inspect the actual
navigation and `/dev/icons` before drawing. Check whether an existing domain or official
integration glyph already owns the concept.

1. Pick one domain metaphor. Sketch the outline at 16px alongside neighboring glyphs. Keep
   Intent's original filled Agent geometry unchanged unless the user explicitly requests it.
2. Edit the canonical paths in
   `apps/ui/src/components/icons/facet-icon-data.ts`. Use simple absolute SVG commands on the
   24-unit artboard. Keep the outline's default weight, square caps, and mitered joins. Favor
   a few deliberate paths over detail; adjust optical spacing instead of adding weight.
3. Export a named component from `facet-icons.tsx`; add a labeled gallery entry. Reuse that
   component through the shared navigation model and the domain's entity fallbacks/mastheads.
   Preserve uploaded images, backend-selected icons, and official logos.
4. Inspect `/dev/icons` in light and dark at 16, 20, 24, and 32px, then inspect desktop and
   mobile navigation, Settings, search, and an entity view. Compare against the approved
   Agent master and current neighbors, not a fresh generated style board.
5. From the repository root, export editable SVGs with
   `node apps/ui/scripts/export-facet-icons.mjs` (Node >=22.18); an optional argument chooses
   the output directory. The default `output/facet-icons` is local proof material. Gallery
   SVG download and the batch exporter use the same serializer. Do not maintain a second
   manually edited SVG collection.
6. Run the Facet and affected navigation/avatar tests, UI lint/typecheck/format, design lint,
   and `just check-okf` if knowledge changed. Update the philosophy only when the design
   contract changes, and capture real UI evidence before shipping.

## Optional image-generation brief

Image generation is for exploring a new metaphor, not producing the final menu asset. Supply
an exported Agent master and a current UI screenshot as references when available:

> Design one new Everruns Facet domain icon for [DOMAIN / PURPOSE]. Match the existing Slate
> interface: quiet editorial geometry, monochrome vector drawing, sharp corners, square line
> terminals, mitered joins, sparse details, generous negative space. Use a 24-unit grid and
> 1.75-unit supporting outline. It must read at 16px next to the original solid Intent Agent
> mark. Draw a recognizable [METAPHOR], optically balanced as if carefully constructed by a
> human in Adobe Illustrator. Flat white background, no gradient, shadows, texture, glossy 3D,
> mascot, face, decorative sparkle, lettering, or surrounding UI. Show an enlarged construction
> and actual menu-size silhouette. Do not redesign the Agent or copy an external product mark.

Redraw the chosen idea in the path masters. Avoid automatic raster tracing. All paths must be
reviewable and static; no arbitrary SVG from an API or database enters this family.
