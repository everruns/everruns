# Diagram Specification

Style guide for technical diagrams in documentation. All diagrams follow the Everruns brand system (see `specs/brand.md`) and are hand-authored SVG.

Adopted from [`everruns/landing/specs/diagrams.md`](https://github.com/everruns/landing/blob/main/specs/diagrams.md).

## Placement

- **File format**: SVG
- **Location**: `docs/images/<category>/<diagram-name>.svg`
- **Embedding**: `![alt text](../images/<category>/<diagram-name>.svg)` in markdown

## Dimensions

- **Width**: 800px (standard). Use `viewBox` so the SVG scales responsively.
- **Height**: Varies. Keep it tight - no empty space at the bottom. Typical range 300-500px.
- **Fill**: Set `fill="none"` on the root `<svg>` element. Add an explicit white background rect.

## Colors

Grayscale palette with Navy as the only accent. No gradients. No other colors.

| Element                      | Hex       | Brand name |
| ---------------------------- | --------- | ---------- |
| Background                   | `#FFFFFF` | White      |
| Box fill                     | `#F5F5F5` | Smoke      |
| Box stroke                   | `#0A0A0A` | Obsidian   |
| Primary text                 | `#0A0A0A` | Obsidian   |
| Secondary text               | `#404040` | Slate      |
| Muted text / section headers | `#A0A0A0` | Silver     |
| Primary arrows               | `#0A1636` | Navy       |
| Step badges (fill)           | `#0A1636` | Navy       |
| Step badge text              | `#FFFFFF` | White      |
| Secondary connectors         | `#A0A0A0` | Silver     |
| Annotation box stroke        | `#404040` | Slate      |

## Typography

Each SVG embeds its own `<style>` block. No external CSS.

```xml
<style>
  text { font-family: 'Geist', 'Inter', system-ui, sans-serif; }
  .label { font-size: 13px; fill: #0A0A0A; font-weight: 600; }
  .sublabel { font-size: 11px; fill: #404040; font-weight: 400; }
  .header-text { font-size: 11px; fill: #A0A0A0; font-weight: 400; letter-spacing: 0.08em; text-transform: uppercase; }
  .mono { font-family: 'Geist Mono', 'SF Mono', monospace; font-size: 10px; fill: #404040; }
  .step-num { font-size: 10px; fill: #FFFFFF; font-weight: 600; }
  .arrow-label { font-size: 10px; fill: #0A1636; font-weight: 500; }
</style>
```

| Class          | Purpose                    | Size           | Weight | Color    |
| -------------- | -------------------------- | -------------- | ------ | -------- |
| `.label`       | Box title                  | 13px           | 600    | Obsidian |
| `.sublabel`    | Box description            | 11px           | 400    | Slate    |
| `.header-text` | Category label above a box | 11px uppercase | 400    | Silver   |
| `.mono`        | Technical details, code    | 10px monospace | 400    | Slate    |
| `.step-num`    | Number inside a step badge | 10px           | 600    | White    |
| `.arrow-label` | Text next to an arrow      | 10px           | 500    | Navy     |

## Geometry rules

- **0px radius on everything**. No rounded corners, ever. This matches the brand.
- **Box stroke**: 1px solid Obsidian
- **Arrow stroke**: 1.5px solid Navy
- **Arrow heads**: Filled Navy triangle via `<polygon>`, 10px wide
- **Connectors** (secondary links): 1px Silver, dashed `stroke-dasharray="3 3"`
- **Annotation boxes** (detail callouts): 1px Slate, dashed `stroke-dasharray="4 3"`, White fill
- **Step badges**: 18x18px Navy-filled rect with centered white number

## Layout principles

1. **Left-to-right** for the primary flow. Top-to-bottom for secondary flows.
2. **Generous spacing** between boxes. Cramped diagrams are hard to read at mobile sizes.
3. **Step badges** mark the sequence. Place them on the arrow, not inside the box.
4. **Annotations** (like header contents or data formats) float near the relevant arrow, connected by a dashed Silver line.
5. **Section headers** (uppercase Silver text) sit above boxes to categorize them.

## Building blocks

Copy-paste these snippets when constructing a diagram.

### Entity box

```xml
<rect x="40" y="60" width="200" height="100" fill="#F5F5F5" stroke="#0A0A0A" stroke-width="1"/>
<text x="140" y="50" text-anchor="middle" class="header-text">CATEGORY</text>
<text x="140" y="100" text-anchor="middle" class="label">Entity Name</text>
<text x="140" y="120" text-anchor="middle" class="sublabel">Short description</text>
<text x="140" y="140" text-anchor="middle" class="mono">Technical detail</text>
```

### Arrow with step badge

```xml
<line x1="240" y1="95" x2="555" y2="95" stroke="#0A1636" stroke-width="1.5"/>
<polygon points="555,90 565,95 555,100" fill="#0A1636"/>
<rect x="370" y="74" width="18" height="18" fill="#0A1636"/>
<text x="379" y="87" text-anchor="middle" class="step-num">1</text>
<text x="395" y="87" class="arrow-label">Description of this step</text>
```

### Dashed connector

```xml
<line x1="400" y1="105" x2="400" y2="170" stroke="#A0A0A0" stroke-width="1" stroke-dasharray="3 3"/>
```

### Annotation box

```xml
<rect x="280" y="170" width="240" height="90" fill="#FFFFFF" stroke="#404040" stroke-width="1" stroke-dasharray="4 3"/>
<text x="400" y="193" text-anchor="middle" class="sublabel" font-weight="500">Box title</text>
<text x="400" y="213" text-anchor="middle" class="mono">Detail line 1</text>
<text x="400" y="228" text-anchor="middle" class="mono">Detail line 2</text>
```

### Result badge

```xml
<rect x="596" y="380" width="128" height="30" fill="#FFFFFF" stroke="#0A1636" stroke-width="1.5"/>
<rect x="596" y="380" width="18" height="30" fill="#0A1636"/>
<text x="605" y="400" text-anchor="middle" class="step-num">3</text>
<text x="668" y="399" text-anchor="middle" class="arrow-label">Outcome text</text>
```

## Generation hints

1. Start with the flow, not the layout. Write steps as a numbered list first.
2. Place the primary flow left-to-right at the top. Secondary flows below.
3. Begin with the SVG template. Replace `{HEIGHT}` once you know the content height.
4. Position boxes first, then connect with arrows. Adjust viewBox height last.
5. Use `text-anchor="middle"` and center x-coordinates inside boxes.
6. Test at small sizes. Make sure 10px text stays legible at ~400px wide.
7. Keep it to 3-5 boxes and 2-4 arrows. Split into two diagrams if needed.
8. No decorative elements. Every element should carry information.

## SVG template

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 {HEIGHT}" fill="none">
  <style>
    text { font-family: 'Geist', 'Inter', system-ui, sans-serif; }
    .label { font-size: 13px; fill: #0A0A0A; font-weight: 600; }
    .sublabel { font-size: 11px; fill: #404040; font-weight: 400; }
    .header-text { font-size: 11px; fill: #A0A0A0; font-weight: 400; letter-spacing: 0.08em; text-transform: uppercase; }
    .mono { font-family: 'Geist Mono', 'SF Mono', monospace; font-size: 10px; fill: #404040; }
    .step-num { font-size: 10px; fill: #FFFFFF; font-weight: 600; }
    .arrow-label { font-size: 10px; fill: #0A1636; font-weight: 500; }
  </style>
  <rect width="800" height="{HEIGHT}" fill="#FFFFFF"/>
</svg>
```
