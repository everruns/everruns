# OpenUI — Generative UI Integration

## Purpose

Enables agents to generate rich interactive UI components (charts, tables, forms, cards, dashboards) directly in chat using the [OpenUI](https://openui.com) framework.

## Architecture

```
┌─────────────────┐     ┌──────────────────┐     ┌───────────────────────┐
│  openui crate   │────▶│ openui capability │────▶│   LLM system prompt   │
│ (components +   │     │ (in everruns-core)│     │  with OpenUI Lang     │
│  prompt gen)    │     │                  │     │  syntax + components  │
└─────────────────┘     └──────────────────┘     └───────────────────────┘
                                                          │
                                                          ▼
                                                 ┌─────────────────┐
                                                 │   LLM response  │
                                                 │  with ```openui │
                                                 │   code blocks   │
                                                 └────────┬────────┘
                                                          │
                                                          ▼
                                                 ┌─────────────────┐
                                                 │   Chat UI       │
                                                 │  MessageContent │
                                                 │  splits blocks  │
                                                 └────────┬────────┘
                                                          │
                                          ┌───────────────┴───────────────┐
                                          ▼                               ▼
                                 ┌─────────────────┐             ┌──────────────┐
                                 │ StreamdownMessage│             │ OpenUIBlock   │
                                 │ (markdown text)  │             │ (Renderer +   │
                                 │                  │             │  react-ui)    │
                                 └─────────────────┘             └──────────────┘
```

## Detection Mechanism

LLM output containing OpenUI Lang is wrapped in ` ```openui ``` ` fenced code blocks. The system prompt instructs the LLM to use this format. The UI splits message text on these blocks and renders them with the OpenUI `<Renderer>` component.

This approach:
- Coexists with markdown text (explanations before/after)
- Uses standard fenced code block syntax (familiar to LLMs)
- Is reliable to parse with a simple regex
- Degrades gracefully (shows as a code block if rendering fails)

## Crate: `everruns-openui`

Path: `crates/openui/`

Static Rust definitions of all OpenUI components and a prompt generator. No runtime parsing — the crate only produces the system prompt text that instructs LLMs to generate OpenUI Lang.

### Key types

- `ComponentDef` — name, props (ordered), description
- `PropDef` — name, type annotation, optional flag
- `ComponentGroup` — logical grouping for prompt organization
- `Library` — root component, all components, groups
- `PromptOptions` — custom preamble, additional rules, examples

### Prompt generation

`generate_prompt(library, options)` produces the system prompt with:
1. Preamble (instructs use of ` ```openui ``` ` blocks)
2. Syntax rules (9 rules about identifiers, expressions, references)
3. Component signatures (auto-generated from component definitions)
4. Streaming/hoisting rules
5. Important rules
6. Optional examples and additional rules

Ref: `packages/react-lang/src/parser/prompt.ts` in the upstream repo.

### Component library

~55 components matching `@openuidev/react-ui` chat library:

| Group | Components |
|-------|-----------|
| Content | CardHeader, TextContent, MarkDownRenderer, Callout, TextCallout, Image, ImageBlock, ImageGallery, CodeBlock, Separator |
| Tables | Table, Col |
| Charts (2D) | BarChart, LineChart, AreaChart, RadarChart, HorizontalBarChart, Series |
| Charts (1D) | PieChart, RadialChart, SingleStackedBarChart, Slice |
| Charts (Scatter) | ScatterChart, ScatterSeries, Point |
| Forms | Form, FormControl, Label, Input, TextArea, Select, SelectItem, DatePicker, Slider, CheckBoxGroup, CheckBoxItem, RadioGroup, RadioItem, SwitchGroup, SwitchItem |
| Buttons | Button, Buttons |
| Lists & Follow-ups | ListBlock, ListItem, FollowUpBlock, FollowUpItem |
| Sections | SectionBlock, SectionItem |
| Layout | Stack, Card, Tabs, TabItem, Accordion, AccordionItem, Steps, StepsItem, Carousel |
| Data Display | TagBlock, Tag |

## Capability: `openui`

ID: `openui`
Feature: `"openui"`

Registered as a built-in capability. When enabled on an agent, appends the OpenUI system prompt. No tools — the capability only contributes to the system prompt.

## UI Integration

### npm packages

- `@openuidev/react-lang` — Parser and `<Renderer>` component
- `@openuidev/react-ui` — Pre-built component library (~50 components)

### Key UI files

- `apps/ui/src/lib/openui-utils.ts` — `splitOpenUIBlocks()`, `hasOpenUIBlocks()`
- `apps/ui/src/components/chat/openui-renderer.tsx` — `<OpenUIBlock>` wrapper
- `apps/ui/src/components/chat/message-content.tsx` — `<MessageContent>` splits text into markdown + openui segments

### Streaming

The OpenUI `<Renderer>` supports progressive rendering via its `isStreaming` prop. During streaming:
- Forward references (hoisting) allow the root component to render immediately
- Child definitions fill in as they stream
- The parser re-parses on each chunk

## Example

User prompt: "Show me monthly revenue data"

LLM response:
````
Here's the revenue breakdown:

```openui
root = Card([header, chart])
header = CardHeader("Monthly Revenue", "Q1 2024")
chart = BarChart(labels, [series1])
labels = ["Jan", "Feb", "Mar"]
series1 = Series("Revenue", [45000, 52000, 61000])
```

The data shows steady growth across Q1.
````

This renders as a Card with a header and bar chart, with markdown text above and below.
