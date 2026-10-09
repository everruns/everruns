//! Page reading for the `browser` tool: the accessibility tree with element
//! refs, `find`, page text, `form_input` and ref resolution.
//!
//! THREAT[TM-TOOL-060]: refs name Chrome's backend DOM node ids, kept on the Everruns
//! side in session storage, never in the page. A page script cannot see or
//! move them, so a ref the model read as "Cancel" cannot be repointed at
//! "Buy". The table is tied to the main frame's loader id: a navigation makes
//! every ref stale, and a node the page removed fails to resolve, so a stale
//! ref is reported rather than clicked somewhere else.
//!
//! Decision: the tree comes from `Accessibility.getFullAXTree` (roles and
//! names as assistive technology sees them) and visibility from one
//! `DOMSnapshot.captureSnapshot` (layout boxes for every node), two calls for
//! the whole page instead of one per element.
//!
//! THREAT[TM-TOOL-060]: scripts the tool runs (`get_page_text`, `form_input`) run in an
//! isolated world, so a page that overrode `innerText` or the `value` setter
//! in its own world cannot change what they read or write.
//!
//! Decision: `find` ranks elements by how many words of the query appear in
//! their role, name, value or description. It needs no model call; a query
//! that names what the element says or does finds it.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use everruns_contracts::runtime::browser_use::{
    DEFAULT_READ_DEPTH, MAX_FIND_MATCHES, MAX_PAGE_OUTPUT_CHARS, ReadFilter,
};
use everruns_contracts::runtime::computer_use::DisplaySize;

use crate::browserless::cdp::CdpSession;

/// Name of the isolated world the tool's scripts run in.
const WORLD_NAME: &str = "everruns_browser_use";
/// Refs one table keeps; older refs past this are dropped.
const MAX_REFS: usize = 5_000;
/// Longest element name in a line.
const MAX_NAME_CHARS: usize = 200;

/// Refs of one tab's current document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RefTable {
    /// Main frame loader id the refs belong to.
    pub document: String,
    pub next: u32,
    /// `ref_N` -> backend DOM node id.
    pub refs: BTreeMap<String, i64>,
}

impl RefTable {
    /// The table for `document`: kept when it is the same document, else new.
    pub fn for_document(self, document: &str) -> Self {
        if self.document == document {
            self
        } else {
            Self {
                document: document.to_string(),
                next: 1,
                refs: BTreeMap::new(),
            }
        }
    }

    fn ref_for(&mut self, node: i64, reverse: &mut HashMap<i64, String>) -> String {
        if let Some(existing) = reverse.get(&node) {
            return existing.clone();
        }
        let name = format!("ref_{}", self.next);
        self.next += 1;
        if self.refs.len() >= MAX_REFS
            && let Some(oldest) = self.refs.keys().next().cloned()
        {
            self.refs.remove(&oldest);
        }
        self.refs.insert(name.clone(), node);
        reverse.insert(node, name.clone());
        name
    }
}

/// The message for a ref that no longer names an element.
pub(crate) fn stale_ref(reference: &str) -> String {
    format!(
        "{reference} is stale or not found on the current page. Re-read the page to get fresh references."
    )
}

/// One accessibility node, flattened from CDP's shape.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct AxNode {
    pub id: String,
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub ignored: bool,
    pub role: String,
    pub name: String,
    pub value: String,
    pub description: String,
    pub backend: Option<i64>,
    /// States worth showing: checked, selected, expanded, disabled, ...
    pub states: Vec<String>,
    pub focusable: bool,
}

/// The parsed accessibility tree and each node's visibility.
#[derive(Debug, Default)]
pub(crate) struct PageTree {
    pub nodes: HashMap<String, AxNode>,
    pub order: Vec<String>,
    pub root: Option<String>,
    /// Backend node ids with a layout box in the viewport.
    pub in_viewport: HashSet<i64>,
    /// Backend node ids with any layout box.
    pub rendered: HashSet<i64>,
}

fn ax_text(node: &Value, key: &str) -> String {
    node.get(key)
        .and_then(|v| v.get("value"))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

/// Parse `Accessibility.getFullAXTree` nodes.
pub(crate) fn parse_ax_nodes(nodes: &[Value]) -> (HashMap<String, AxNode>, Vec<String>) {
    let mut map = HashMap::new();
    let mut order = Vec::new();
    for raw in nodes {
        let Some(id) = raw.get("nodeId").and_then(Value::as_str) else {
            continue;
        };
        let mut states = Vec::new();
        let mut focusable = false;
        for property in raw
            .get("properties")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let name = property
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let value = property.get("value").and_then(|v| v.get("value"));
            match (name, value) {
                ("focusable", Some(Value::Bool(true))) => focusable = true,
                ("checked" | "pressed", Some(Value::String(state)))
                    if state == "true" || state == "mixed" =>
                {
                    states.push(format!("{name}={state}"))
                }
                ("checked" | "pressed" | "selected" | "expanded", Some(Value::Bool(true))) => {
                    states.push(name.to_string())
                }
                ("disabled" | "required" | "readonly", Some(Value::Bool(true))) => {
                    states.push(name.to_string())
                }
                ("expanded", Some(Value::Bool(false))) => states.push("collapsed".to_string()),
                ("level", Some(level)) => states.push(format!("level={level}")),
                _ => {}
            }
        }
        let node = AxNode {
            id: id.to_string(),
            parent: raw
                .get("parentId")
                .and_then(Value::as_str)
                .map(str::to_string),
            children: raw
                .get("childIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            ignored: raw.get("ignored").and_then(Value::as_bool).unwrap_or(false),
            role: ax_text(raw, "role"),
            name: ax_text(raw, "name"),
            value: ax_text(raw, "value"),
            description: ax_text(raw, "description"),
            backend: raw.get("backendDOMNodeId").and_then(Value::as_i64),
            states,
            focusable,
        };
        order.push(node.id.clone());
        map.insert(node.id.clone(), node);
    }
    (map, order)
}

/// Layout boxes from `DOMSnapshot.captureSnapshot`: which backend nodes are
/// rendered, and which of those intersect the viewport.
pub(crate) fn parse_layout(
    snapshot: &Value,
    viewport: DisplaySize,
) -> (HashSet<i64>, HashSet<i64>) {
    let mut rendered = HashSet::new();
    let mut visible = HashSet::new();
    let Some(document) = snapshot
        .get("documents")
        .and_then(Value::as_array)
        .and_then(|documents| documents.first())
    else {
        return (rendered, visible);
    };
    let backend_ids: Vec<i64> = document
        .pointer("/nodes/backendNodeId")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|id| id.as_i64().unwrap_or(-1))
        .collect();
    let scroll_x = document
        .get("scrollOffsetX")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let scroll_y = document
        .get("scrollOffsetY")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let indices = document
        .pointer("/layout/nodeIndex")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let bounds = document
        .pointer("/layout/bounds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (width, height) = (f64::from(viewport.width), f64::from(viewport.height));
    for (index, rect) in indices.iter().zip(bounds.iter()) {
        let Some(backend) = index
            .as_u64()
            .and_then(|index| backend_ids.get(index as usize))
            .copied()
        else {
            continue;
        };
        let rect: Vec<f64> = rect
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_f64)
            .collect();
        let [x, y, w, h] = rect[..] else {
            continue;
        };
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        rendered.insert(backend);
        let (left, top) = (x - scroll_x, y - scroll_y);
        if left < width && top < height && left + w > 0.0 && top + h > 0.0 {
            visible.insert(backend);
        }
    }
    (rendered, visible)
}

/// Roles that only group or decorate; their children are read in their place.
fn is_structural(role: &str) -> bool {
    matches!(
        role,
        "generic"
            | "none"
            | "presentation"
            | "InlineTextBox"
            | "LineBreak"
            | "RootWebArea"
            | "WebArea"
            | "Iframe"
            | "IframePresentational"
            | "group"
            | "Section"
            | "LayoutTable"
            | "LayoutTableRow"
            | "LayoutTableCell"
            | ""
    )
}

/// Roles a person acts on.
fn is_interactive_role(role: &str) -> bool {
    matches!(
        role,
        "button"
            | "link"
            | "textbox"
            | "searchbox"
            | "checkbox"
            | "radio"
            | "combobox"
            | "listbox"
            | "option"
            | "menuitem"
            | "menuitemcheckbox"
            | "menuitemradio"
            | "slider"
            | "spinbutton"
            | "switch"
            | "tab"
            | "treeitem"
            | "ComboBoxSelect"
            | "ComboBoxMenuButton"
            | "TextField"
    )
}

impl PageTree {
    fn interactive(&self, node: &AxNode) -> bool {
        is_interactive_role(&node.role) || (node.focusable && !is_structural(&node.role))
    }

    fn visible(&self, node: &AxNode) -> bool {
        node.backend
            .is_some_and(|id| self.in_viewport.contains(&id))
    }

    fn line(
        &self,
        node: &AxNode,
        table: &mut RefTable,
        reverse: &mut HashMap<i64, String>,
    ) -> String {
        if node.role == "StaticText" {
            return format!("text {}", quoted(&node.name));
        }
        let mut line = format!("{} {}", node.role, quoted(&node.name));
        if !node.value.is_empty() && node.value != node.name {
            line.push_str(&format!(" value={}", quoted(&node.value)));
        }
        for state in &node.states {
            line.push(' ');
            line.push_str(state);
        }
        if let Some(backend) = node.backend {
            line.push_str(&format!(" [{}]", table.ref_for(backend, reverse)));
        }
        line
    }

    /// `read_page` output. Refs it prints are added to `table`.
    pub fn read(
        &self,
        filter: Option<ReadFilter>,
        depth: Option<u32>,
        start: Option<i64>,
        table: &mut RefTable,
    ) -> Result<String, String> {
        let mut reverse: HashMap<i64, String> = table
            .refs
            .iter()
            .map(|(name, id)| (*id, name.clone()))
            .collect();
        let root = match start {
            Some(backend) => self
                .order
                .iter()
                .find(|id| self.nodes[*id].backend == Some(backend))
                .cloned()
                .ok_or_else(|| "the ref's element has no accessibility node".to_string())?,
            None => self.root.clone().ok_or("the page has no document")?,
        };
        let max_depth = depth.unwrap_or(DEFAULT_READ_DEPTH) as usize;
        let mut out = Output::default();
        let mut stack = vec![(root, 0usize, 0usize)];
        while let Some((id, depth, indent)) = stack.pop() {
            if out.full {
                break;
            }
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            let mut child_indent = indent;
            let skip = node.ignored
                || is_structural(&node.role) && node.name.is_empty()
                || node.role == "RootWebArea"
                || node.role == "InlineTextBox";
            if !skip {
                let shown = match filter {
                    Some(ReadFilter::All) => {
                        node.backend.is_none_or(|id| self.rendered.contains(&id))
                    }
                    Some(ReadFilter::Interactive) => self.interactive(node) && self.visible(node),
                    None => {
                        self.visible(node) || node.role == "StaticText" && self.text_visible(node)
                    }
                };
                let redundant = node.role == "StaticText"
                    && node
                        .parent
                        .as_ref()
                        .and_then(|parent| self.nodes.get(parent))
                        .is_some_and(|parent| parent.name.trim() == node.name.trim());
                if shown
                    && !redundant
                    && !(node.role == "StaticText" && node.name.trim().is_empty())
                {
                    let pad = if filter == Some(ReadFilter::Interactive) {
                        String::new()
                    } else {
                        "  ".repeat(indent)
                    };
                    out.push(&format!("{pad}{}", self.line(node, table, &mut reverse)));
                    child_indent = indent + 1;
                }
            }
            if depth + 1 > max_depth {
                continue;
            }
            for child in node.children.iter().rev() {
                stack.push((child.clone(), depth + 1, child_indent));
            }
        }
        Ok(out.finish("Use depth, ref or filter to read part of the page."))
    }

    /// A text node is visible when its parent element is.
    fn text_visible(&self, node: &AxNode) -> bool {
        let mut current = node.parent.as_ref();
        while let Some(id) = current {
            let Some(parent) = self.nodes.get(id) else {
                return false;
            };
            if let Some(backend) = parent.backend {
                return self.in_viewport.contains(&backend);
            }
            current = parent.parent.as_ref();
        }
        false
    }

    /// `find` output: the best matches for `query`, in page order on ties.
    pub fn find(&self, query: &str, table: &mut RefTable) -> String {
        let mut reverse: HashMap<i64, String> = table
            .refs
            .iter()
            .map(|(name, id)| (*id, name.clone()))
            .collect();
        let words = query_words(query);
        let mut scored: Vec<(usize, usize, &AxNode)> = self
            .order
            .iter()
            .enumerate()
            .filter_map(|(position, id)| {
                let node = &self.nodes[id];
                if node.ignored
                    || node.backend.is_none()
                    || node.role == "StaticText"
                    || is_structural(&node.role) && node.name.is_empty()
                    || !node.backend.is_some_and(|id| self.rendered.contains(&id))
                {
                    return None;
                }
                let score = match_score(node, &words);
                (score > 0).then_some((score, position, node))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        if scored.is_empty() {
            return format!(
                "No element matched {}. Try other words, or read_page.",
                quoted(query)
            );
        }
        let mut out = Output::default();
        for (_, _, node) in scored.into_iter().take(MAX_FIND_MATCHES) {
            out.push(&self.line(node, table, &mut reverse));
        }
        out.finish("Narrow the query.")
    }
}

/// Words of a `find` query worth matching.
fn query_words(query: &str) -> Vec<String> {
    const STOP: [&str; 14] = [
        "the", "a", "an", "to", "of", "for", "on", "in", "with", "and", "or", "that", "this", "is",
    ];
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty() && !STOP.contains(word))
        .map(str::to_string)
        .collect()
}

fn match_score(node: &AxNode, words: &[String]) -> usize {
    let text = format!(
        "{} {} {} {}",
        node.name, node.value, node.description, node.role
    )
    .to_lowercase();
    let role_alias = |word: &str| match word {
        "field" | "input" | "box" => {
            matches!(node.role.as_str(), "textbox" | "searchbox" | "combobox")
        }
        "dropdown" | "select" => matches!(node.role.as_str(), "combobox" | "listbox"),
        "toggle" => matches!(node.role.as_str(), "switch" | "checkbox"),
        _ => false,
    };
    words
        .iter()
        .filter(|word| text.contains(word.as_str()) || role_alias(word))
        .count()
        * 2
        + usize::from(
            is_interactive_role(&node.role) && words.iter().any(|w| text.contains(w.as_str())),
        )
}

fn quoted(text: &str) -> String {
    let clean: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    serde_json::to_string(&clean).unwrap_or_else(|_| "\"\"".to_string())
}

#[derive(Default)]
struct Output {
    text: String,
    full: bool,
}

impl Output {
    fn push(&mut self, line: &str) {
        if self.full {
            return;
        }
        if self.text.len() + line.len() + 1 > MAX_PAGE_OUTPUT_CHARS {
            self.full = true;
            return;
        }
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(line);
    }

    fn finish(self, hint: &str) -> String {
        if self.full {
            format!(
                "{}\n[Output cut at {MAX_PAGE_OUTPUT_CHARS} characters. {hint}]",
                self.text
            )
        } else if self.text.is_empty() {
            "(empty)".to_string()
        } else {
            self.text
        }
    }
}

// ============================================================================
// CDP calls
// ============================================================================

/// The main frame's id and loader id.
pub(crate) async fn main_frame(session: &mut CdpSession) -> Result<(String, String), String> {
    let tree = session.send_command("Page.getFrameTree", json!({})).await?;
    let frame = tree
        .pointer("/frameTree/frame")
        .ok_or("the page has no main frame")?;
    let id = frame.get("id").and_then(Value::as_str).unwrap_or_default();
    let loader = frame
        .get("loaderId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok((id.to_string(), loader.to_string()))
}

/// Read the accessibility tree and layout of the attached page.
pub(crate) async fn capture(
    session: &mut CdpSession,
    viewport: DisplaySize,
) -> Result<PageTree, String> {
    session
        .send_command("Accessibility.enable", json!({}))
        .await?;
    let ax = session
        .send_command("Accessibility.getFullAXTree", json!({}))
        .await?;
    let raw = ax
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (nodes, order) = parse_ax_nodes(&raw);
    let root = order.iter().find(|id| nodes[*id].parent.is_none()).cloned();
    let snapshot = session
        .send_command(
            "DOMSnapshot.captureSnapshot",
            json!({ "computedStyles": [] }),
        )
        .await?;
    let (rendered, in_viewport) = parse_layout(&snapshot, viewport);
    Ok(PageTree {
        nodes,
        order,
        root,
        in_viewport,
        rendered,
    })
}

/// The backend node id a ref names, or the stale-ref error.
pub(crate) fn backend_for(
    table: &RefTable,
    document: &str,
    reference: &str,
) -> Result<i64, String> {
    if table.document != document {
        return Err(stale_ref(reference));
    }
    table
        .refs
        .get(reference)
        .copied()
        .ok_or_else(|| stale_ref(reference))
}

/// Scroll a node into view and return its center in viewport pixels.
pub(crate) async fn node_center(
    session: &mut CdpSession,
    backend: i64,
    reference: &str,
    viewport: DisplaySize,
) -> Result<[u32; 2], String> {
    scroll_into_view(session, backend, reference).await?;
    let quads = session
        .send_command("DOM.getContentQuads", json!({ "backendNodeId": backend }))
        .await
        .map_err(|_| stale_ref(reference))?;
    let quad: Vec<f64> = quads
        .pointer("/quads/0")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    if quad.len() != 8 {
        return Err(format!("{reference} is not rendered on the page"));
    }
    let x = (quad[0] + quad[2] + quad[4] + quad[6]) / 4.0;
    let y = (quad[1] + quad[3] + quad[5] + quad[7]) / 4.0;
    if x < 0.0 || y < 0.0 || x >= f64::from(viewport.width) || y >= f64::from(viewport.height) {
        return Err(format!(
            "{reference} is outside the viewport even after scrolling"
        ));
    }
    Ok([x as u32, y as u32])
}

pub(crate) async fn scroll_into_view(
    session: &mut CdpSession,
    backend: i64,
    reference: &str,
) -> Result<(), String> {
    session
        .send_command(
            "DOM.scrollIntoViewIfNeeded",
            json!({ "backendNodeId": backend }),
        )
        .await
        .map(|_| ())
        .map_err(|_| stale_ref(reference))
}

/// A fresh isolated world on the main frame.
async fn isolated_world(session: &mut CdpSession) -> Result<i64, String> {
    let (frame, _) = main_frame(session).await?;
    let world = session
        .send_command(
            "Page.createIsolatedWorld",
            json!({ "frameId": frame, "worldName": WORLD_NAME, "grantUniveralAccess": false }),
        )
        .await?;
    world
        .get("executionContextId")
        .and_then(Value::as_i64)
        .ok_or_else(|| "Page.createIsolatedWorld returned no context".to_string())
}

const PAGE_TEXT: &str = "(() => { const main = document.querySelector('main, [role=main], article') || document.body; return main ? main.innerText : ''; })()";

/// Visible text of the page, main content first.
pub(crate) async fn page_text(session: &mut CdpSession) -> Result<String, String> {
    let context = isolated_world(session).await?;
    let result = session
        .send_command(
            "Runtime.evaluate",
            json!({ "expression": PAGE_TEXT, "contextId": context, "returnByValue": true }),
        )
        .await?;
    let text = result
        .pointer("/result/value")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let mut out = Output::default();
    out.push(
        &text
            .chars()
            .take(MAX_PAGE_OUTPUT_CHARS - 200)
            .collect::<String>(),
    );
    let cut = text.chars().count() > MAX_PAGE_OUTPUT_CHARS - 200;
    let text = out.finish("");
    Ok(if cut {
        format!("{text}\n[Text cut at {MAX_PAGE_OUTPUT_CHARS} characters.]")
    } else {
        text
    })
}

const SET_VALUE: &str = r#"function (value) {
  const el = this;
  const tag = el.tagName;
  if (tag === 'SELECT') {
    const options = Array.from(el.options);
    const want = String(value);
    const option = options.find((o) => o.value === want) || options.find((o) => o.textContent.trim() === want.trim());
    if (!option) return { error: 'no option ' + JSON.stringify(want) + '; options: ' + options.slice(0, 20).map((o) => o.textContent.trim()).join(', ') };
    el.value = option.value;
  } else if (tag === 'INPUT' && (el.type === 'checkbox' || el.type === 'radio')) {
    if (typeof value !== 'boolean') return { error: 'a checkbox or radio button takes true or false' };
    el.checked = value;
  } else if (tag === 'INPUT' || tag === 'TEXTAREA') {
    const proto = tag === 'INPUT' ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, String(value));
  } else if (el.isContentEditable) {
    el.textContent = String(value);
  } else {
    return { error: 'the element is not a form field' };
  }
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return { value: (el.type === 'checkbox' || el.type === 'radio') ? el.checked : (el.value ?? el.textContent) };
}"#;

/// Set a form field's value, firing `input` and `change`.
pub(crate) async fn set_value(
    session: &mut CdpSession,
    backend: i64,
    reference: &str,
    value: &Value,
) -> Result<String, String> {
    let context = isolated_world(session).await?;
    let object = session
        .send_command(
            "DOM.resolveNode",
            json!({ "backendNodeId": backend, "executionContextId": context }),
        )
        .await
        .map_err(|_| stale_ref(reference))?;
    let object_id = object
        .pointer("/object/objectId")
        .and_then(Value::as_str)
        .ok_or_else(|| stale_ref(reference))?
        .to_string();
    let result = session
        .send_command(
            "Runtime.callFunctionOn",
            json!({
                "objectId": object_id,
                "functionDeclaration": SET_VALUE,
                "arguments": [{ "value": value }],
                "returnByValue": true
            }),
        )
        .await?;
    let outcome = result
        .pointer("/result/value")
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(error) = outcome.get("error").and_then(Value::as_str) {
        return Err(format!("{reference}: {error}"));
    }
    Ok(format!(
        "Set {reference} to {}.",
        outcome.get("value").cloned().unwrap_or(Value::Null)
    ))
}

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;
