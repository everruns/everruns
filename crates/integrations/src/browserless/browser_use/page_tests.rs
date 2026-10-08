use super::*;

fn ax(id: &str, parent: Option<&str>, children: &[&str], role: &str, name: &str) -> Value {
    let mut node = json!({
        "nodeId": id,
        "ignored": false,
        "role": { "type": "role", "value": role },
        "name": { "type": "computedString", "value": name },
        "childIds": children,
        "backendDOMNodeId": id.parse::<i64>().unwrap() + 100,
    });
    if let Some(parent) = parent {
        node["parentId"] = json!(parent);
    }
    node
}

/// A form: heading, a name field, a newsletter checkbox, a submit button and
/// a footer link scrolled out of the viewport.
fn form_tree(visible: &[i64]) -> PageTree {
    let mut checkbox = ax("5", Some("2"), &[], "checkbox", "Newsletter");
    checkbox["properties"] = json!([
        { "name": "checked", "value": { "type": "tristate", "value": "true" } },
        { "name": "focusable", "value": { "type": "booleanOrUndefined", "value": true } }
    ]);
    let mut field = ax("4", Some("2"), &[], "textbox", "Full name");
    field["value"] = json!({ "type": "string", "value": "Ada" });
    let raw = vec![
        ax("1", None, &["2", "7"], "RootWebArea", "Signup"),
        ax("2", Some("1"), &["3", "4", "5", "6"], "generic", ""),
        ax("3", Some("2"), &["8"], "heading", "Create account"),
        field,
        checkbox,
        ax("6", Some("2"), &[], "button", "Sign up"),
        ax("7", Some("1"), &[], "link", "Privacy policy"),
        ax("8", Some("3"), &[], "StaticText", "Create account"),
    ];
    let (nodes, order) = parse_ax_nodes(&raw);
    PageTree {
        root: Some("1".into()),
        nodes,
        order,
        in_viewport: visible.iter().copied().collect(),
        rendered: (101..=108).collect(),
    }
}

fn table() -> RefTable {
    RefTable::default().for_document("loader-1")
}

#[test]
fn reads_the_viewport_with_roles_names_states_and_refs() {
    let tree = form_tree(&[102, 103, 104, 105, 106]);
    let mut refs = table();
    let text = tree.read(None, None, None, &mut refs).unwrap();
    assert_eq!(
        text,
        "heading \"Create account\" [ref_1]\n\
         textbox \"Full name\" value=\"Ada\" [ref_2]\n\
         checkbox \"Newsletter\" checked=true [ref_3]\n\
         button \"Sign up\" [ref_4]"
    );
    assert_eq!(refs.refs["ref_4"], 106);

    // Reading again keeps the same refs; a new element gets the next one.
    let all = tree
        .read(Some(ReadFilter::All), None, None, &mut refs)
        .unwrap();
    assert!(all.contains("button \"Sign up\" [ref_4]"), "{all}");
    assert!(all.contains("link \"Privacy policy\" [ref_5]"), "{all}");
}

#[test]
fn interactive_filter_lists_only_visible_controls_flat() {
    let tree = form_tree(&[102, 103, 104, 105, 106]);
    let text = tree
        .read(Some(ReadFilter::Interactive), None, None, &mut table())
        .unwrap();
    assert_eq!(
        text,
        "textbox \"Full name\" value=\"Ada\" [ref_1]\n\
         checkbox \"Newsletter\" checked=true [ref_2]\n\
         button \"Sign up\" [ref_3]"
    );
}

#[test]
fn depth_and_ref_bound_the_read() {
    let tree = form_tree(&[101, 102, 103, 104, 105, 106, 107, 108]);
    let shallow = tree
        .read(Some(ReadFilter::All), Some(1), None, &mut table())
        .unwrap();
    assert_eq!(shallow, "link \"Privacy policy\" [ref_1]");
    let under_heading = tree
        .read(Some(ReadFilter::All), None, Some(103), &mut table())
        .unwrap();
    assert_eq!(under_heading, "heading \"Create account\" [ref_1]");
    assert!(
        tree.read(None, None, Some(999), &mut table())
            .unwrap_err()
            .contains("no accessibility node")
    );
}

#[test]
fn find_ranks_by_query_words_and_reports_no_match() {
    let tree = form_tree(&[]);
    let mut refs = table();
    let found = tree.find("the sign up button", &mut refs);
    assert_eq!(found.lines().next(), Some("button \"Sign up\" [ref_1]"));
    let field = tree.find("name field", &mut refs);
    assert!(
        field.starts_with("textbox \"Full name\" value=\"Ada\" [ref_"),
        "{field}"
    );
    assert!(
        tree.find("checkout total", &mut refs)
            .starts_with("No element matched")
    );
}

#[test]
fn refs_go_stale_with_the_document() {
    let mut refs = table();
    refs.refs.insert("ref_1".into(), 42);
    assert_eq!(backend_for(&refs, "loader-1", "ref_1"), Ok(42));
    assert_eq!(
        backend_for(&refs, "loader-2", "ref_1"),
        Err(stale_ref("ref_1"))
    );
    assert_eq!(
        backend_for(&refs, "loader-1", "ref_9"),
        Err(
            "ref_9 is stale or not found on the current page. Re-read the page to get fresh references."
                .into()
        )
    );
    let fresh = refs.for_document("loader-2");
    assert!(fresh.refs.is_empty());
    assert_eq!(fresh.next, 1);
}

#[test]
fn layout_splits_rendered_from_in_viewport() {
    let snapshot = json!({ "documents": [{
        "scrollOffsetX": 0, "scrollOffsetY": 500,
        "nodes": { "backendNodeId": [10, 11, 12, 13] },
        "layout": {
            "nodeIndex": [0, 1, 2, 3],
            "bounds": [[0, 0, 800, 2000], [10, 600, 100, 20], [10, 100, 100, 20], [10, 700, 0, 0]]
        }
    }]});
    let (rendered, visible) = parse_layout(
        &snapshot,
        DisplaySize {
            width: 800,
            height: 600,
        },
    );
    assert_eq!(rendered, HashSet::from([10, 11, 12]));
    // 12 sits above the scrolled viewport; 13 has no box.
    assert_eq!(visible, HashSet::from([10, 11]));
}

#[test]
fn long_output_is_cut_with_a_hint() {
    let mut out = Output::default();
    let line = "x".repeat(1000);
    for _ in 0..60 {
        out.push(&line);
    }
    let text = out.finish("Use depth.");
    assert!(text.len() <= MAX_PAGE_OUTPUT_CHARS + 100);
    assert!(text.ends_with("characters. Use depth.]"));
    assert_eq!(Output::default().finish(""), "(empty)");
}
