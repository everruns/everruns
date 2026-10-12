-- 212 bound App-era AG-UI and Public Chat threads to every channel of the
-- session's App when the session had no channel (channel_id IS NULL). Those
-- sessions are exactly the ones 137 left unattributed because their App had
-- several doors; their `ag_ui:app:` / `public_chat:app:` tags carry the App's
-- alias, which every channel of that App shares, so they cannot say which
-- door the conversation came through. 212 fanned one session out under the
-- same thread key on each such channel.
--
-- Recover only structurally unambiguous ownership: the transport tag already
-- says the thread came through a channel of that type, so an App with a single
-- channel of that type has one possible owner. Every other binding to a
-- channel-less session is removed; that conversation starts over rather than
-- being guessed into a channel (knowledge/runtime-resources/
-- session-source-and-facets.md: derive structurally or leave unknown).
--
-- Runtime bindings only ever point at sessions the host created with their
-- channel set, so a channel-less session's binding can only have come from
-- 212. Sessions with a channel keep every binding to that channel. FCP is
-- untouched: 212 required its channel-specific `fcp:endpoint:` tag.

DELETE FROM channel_bindings AS binding
USING agent_channels AS channel, sessions AS session
WHERE binding.channel_id = channel.id
  AND binding.session_id = session.id
  AND channel.channel_type IN ('ag_ui', 'public_chat')
  AND session.channel_id IS NULL
  AND (
        channel.app_id IS NULL
     OR session.app_id IS DISTINCT FROM channel.app_id
     OR (
            SELECT COUNT(*)
            FROM agent_channels AS sibling
            WHERE sibling.app_id = channel.app_id
              AND sibling.channel_type = channel.channel_type
        ) > 1
  );
