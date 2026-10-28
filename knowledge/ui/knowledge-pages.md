---
type: Decision
title: "Knowledge Pages"
description: "Why Knowledge is a tabbed page like Sandboxes, and why a collection is browsed with the same file browser as a session's filesystem."
tags:
  - everruns
  - ui
  - knowledge
  - files
---

# Knowledge Pages

## Abstract

A Knowledge collection is an indexed filesystem: agents mount it, list it, read it and
grep it, and indexing only adds passage search on top
([Unified Knowledge](../runtime-resources/knowledge.md)). The UI shows it that way. The
landing page is a tabbed page like Sandboxes, and a collection opens on the same tree and
viewer a session's Filesystem tab uses. Source: `apps/ui/src/app/(main)/knowledge/`, the
shared browser in `apps/ui/src/components/files/`, and the collection adapter in
`apps/ui/src/components/knowledge/knowledge-file-source.ts`. The pattern is documented as the file browser layout on the
[design system pages](design-system.md) (`docs/design-system/layouts.mdx`).

## Decisions

### The landing page is tabbed like Sandboxes

`/knowledge` lists collections; `/knowledge/accounts` lists the organization's search
accounts (Turbopuffer indexers). Each tab is its own path through `tabFromPath`/`tabHref`,
with the shell (breadcrumb, masthead, `SectionTabs`) as the tabs' layout, the same structure
as `sandboxes/(tabs)`. Collections are `EntityCard`s showing where files come from, the
indexing mode, file counts and last sync. "New collection" and "Add search account" open
dialogs from the masthead instead of expanding forms inline. Git sources need no entry on
Accounts: private GitHub repositories read through a service identity's GitHub
connection, chosen when the source is connected.

### A collection is a filesystem first

`/knowledge/{id}` has Files (default), Search, Sources and Settings tabs. Files is the
session Filesystem pane: a folder tree on the left and the file viewer on the right, with
the same previews (Markdown, code, images, PDF). The selected file is the `?file=` query
param, so search results and shared links open straight to it. Per-file controls ("Agent
can read", "Include in index") sit in a footer under the viewer. Search runs the passage
search agents use and explains how to turn indexing on when it is off.

### One browser, many filesystems

`FileBrowser` and `FileViewer` take a `FileSource` (list a folder, read a file, a raw
download URL, optional writes) instead of a workspace id. A source without writes renders
read-only, which is how the session recording (EVE-854), the built-in docs and files from
connected sources stay unchangeable. Managed files in a collection get create, edit and
delete; the collection has no `mkdir`, since its folders exist only through their files,
so the browser hides "New folder" for it.

### The tree loads one folder at a time

The file catalog endpoint takes `tree=true` to return the immediate children of a folder:
its files plus one entry per subfolder. The query is a loose index scan over the
`knowledge_files_tree` index: a subfolder costs one index probe however many files it
holds, so the root of a very large collection lists as fast as a small folder. Without it,
the tree would have to page the whole recursive catalog into the browser.

### Page-aware Chat follows each tab

The adopted workspace shares [the page-aware Chat contract](chat-mode.md).
Collection settings, source connection and search-account metadata keep their
typed drafts; explicit submit controls persist them. Clean settings follow server
refreshes, while dirty work is preserved and conflicting saves are blocked.
Displayed collection/source/search state and visible tree entries have bounded
metadata projections, without file bodies, search excerpts, credential bindings or
raw source errors. File-write and upload controls stay manual. A split workspace
stacks the file browser according to the available page pane, independent of the
outer viewport.

### Lifecycle is a dot and a label

Sync, indexing and extraction state use the shared `EntityStatus` dot and label
(`components/ui/entity-status.tsx`), per `apps/ui/DESIGN.md`. It takes an explicit tone and
label for states outside the entity lifecycle; `components/knowledge/knowledge-status.ts`
maps Knowledge states to them. Indexing mode is a configuration label and stays a badge.

## Dismissed alternatives

* **A flat list of full paths with a prefix filter.** The previous page. It does not
  scale past a few hundred files and does not match how agents see the collection.
* **Building folders client-side from the recursive catalog.** Correct only after every
  page of a large collection has loaded.
* **A Knowledge-specific tree component.** It would drift from the session browser; the
  `FileSource` adapter keeps one implementation.
