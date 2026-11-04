# Shared chat renderer

Console chat and Work Chat consume this private source package through local
file dependencies. Keep it inside the console Docker context; the Work Chat
Dockerfile copies it and the shared dependency-security scripts explicitly.

`MessageWidgets` renders A2UI 1.0, saved nested A2UI trees and OpenUI. Pass a
stable message ID, a Markdown renderer, and the host's existing message callback.
Omit the callback for read-only content. `GenerativeUIConversation` can scope
several messages and expose `GenerativeUISurface` targets for embedding hosts;
the shipped chats advertise inline surfaces only.

The A2UI catalog and state engine come from the pinned CopilotKit standard UI.
The provider reads `/generative-ui/info`, which advertises no agents and disables
telemetry. The transport bridge uses Everruns callbacks; it never starts a
CopilotKit agent. Generated HTML has an additional no-network iframe policy.

Run `pnpm run test:generative-ui` in `apps/ui` to check fence parsing, lifecycle,
limits, compatibility and prompt freshness. To refresh the Rust prompt after a
catalog upgrade, run `node generative-ui/scripts/export-catalog.mjs` there.
Both hosts have browser tests using the 69-component fixtures. The console also
has a development-only gallery at `/dev/generative-ui`.

The public docs [component gallery](../../../docs/features/generative-ui-gallery.mdx)
uses the same authored fixtures, with light and dark screenshots and expandable
A2UI source. Regenerate screenshots with the console dev server running:
`node scripts/capture-generative-ui-gallery.mjs http://localhost:9120` from
`apps/ui` (install docs dependencies for Sharp first). External maps and media
have source-only examples so an unavailable provider cannot produce a broken
public preview.

The owning behavioral contract is [A2UI](../../../knowledge/ui/a2ui.md).
Upstream fixture and generated-catalog attribution is retained in
`COPILOTKIT-LICENSE` and the Rust catalog's `standard-catalog.LICENSE`.
