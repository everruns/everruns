DO $$
DECLARE
    default_id UUID;
    runtime_principal UUID;
BEGIN
    IF NOT EXISTS(SELECT 1 FROM payment_accounts WHERE id='00000000-0000-0000-0000-000000000092' AND owner_type='virtual_user' AND owner_id='identity_00000000000000000000000000000061' AND credential_encrypted=decode('beef','hex')) THEN RAISE EXCEPTION 'Payment account owner lost'; END IF;
    IF NOT EXISTS(SELECT 1 FROM github_apps WHERE id='00000000-0000-0000-0000-000000000091' AND virtual_user_id='00000000-0000-0000-0000-000000000061' AND private_key_encrypted=decode('cafe','hex')) THEN RAISE EXCEPTION 'GitHub App credential owner changed'; END IF;
    IF NOT EXISTS(SELECT 1 FROM payment_policies WHERE subject_type='virtual_user' AND subject_id='identity_00000000000000000000000000000061') THEN RAISE EXCEPTION 'Identity payment policy lost'; END IF;
    SELECT virtual_user_id INTO STRICT default_id FROM virtual_user_bindings WHERE org_id=1 AND management_user_id='00000000-0000-0000-0000-000000000011';
    SELECT id INTO STRICT runtime_principal FROM principals WHERE org_id=1 AND kind='virtual_user' AND subject_id=default_id;
    IF (SELECT count(*) FROM virtual_users WHERE usage='end_user')<>4 THEN RAISE EXCEPTION 'Missing per-organization or external runtime accounts'; END IF;
    IF (SELECT count(*) FROM virtual_user_connections)<>2 THEN RAISE EXCEPTION 'Credentials duplicated or missing'; END IF;
    IF NOT EXISTS(SELECT 1 FROM virtual_user_connections WHERE id='00000000-0000-0000-0000-000000000072' AND virtual_user_id=default_id AND access_token_encrypted=decode('1234','hex')) THEN RAISE EXCEPTION 'Single-organization connection changed'; END IF;
    IF NOT EXISTS(SELECT 1 FROM virtual_user_connections WHERE id='00000000-0000-0000-0000-000000000071' AND virtual_user_id='00000000-0000-0000-0000-000000000061' AND access_token_encrypted=decode('abcd','hex')) THEN RAISE EXCEPTION 'Service connection changed'; END IF;
    IF NOT EXISTS(SELECT 1 FROM pending_user_connections WHERE id='00000000-0000-0000-0000-000000000073' AND access_token_encrypted=decode('5678','hex')) THEN RAISE EXCEPTION 'Ambiguous credential was lost or copied'; END IF;
    IF NOT EXISTS(SELECT 1 FROM sessions WHERE id='00000000-0000-0000-0000-000000000041' AND owner_principal_id=runtime_principal AND archived_at IS NOT NULL AND 'platform-chat-starter'=ANY(tags)) THEN RAISE EXCEPTION 'Chat history identity or archive changed'; END IF;
    IF NOT EXISTS(SELECT 1 FROM session_participants WHERE id='00000000-0000-0000-0000-000000000051' AND principal_id=runtime_principal) THEN RAISE EXCEPTION 'Participant identity changed'; END IF;
    IF NOT EXISTS(SELECT 1 FROM pinned_sessions WHERE session_id='00000000-0000-0000-0000-000000000041' AND user_id=default_id) THEN RAISE EXCEPTION 'Pin lost'; END IF;
    IF NOT EXISTS(SELECT 1 FROM virtual_user_bindings WHERE provider='slack' AND realm='T123' AND subject='U123' AND virtual_user_id='00000000-0000-0000-0000-000000000013') THEN RAISE EXCEPTION 'External speaker mapping changed'; END IF;
    IF NOT EXISTS(SELECT 1 FROM leased_resources WHERE id='00000000-0000-0000-0000-000000000081' AND owner_user_id=(SELECT virtual_user_id FROM virtual_user_bindings WHERE org_id=1 AND management_user_id='00000000-0000-0000-0000-000000000012') AND metadata->>'connection_migration_pending'='true' AND pending_connection_id='00000000-0000-0000-0000-000000000073') THEN RAISE EXCEPTION 'Pending cleanup credential provenance lost'; END IF;
    -- Archived starters still arbitrate stale clients after ownership rekeying.
    BEGIN
        INSERT INTO sessions(org_id,owner_principal_id,workspace_id,source,tags) VALUES(1,runtime_principal,'00000000-0000-0000-0000-000000000032','chat',ARRAY['platform-chat-starter']);
        RAISE EXCEPTION 'Duplicate archived starter accepted';
    EXCEPTION WHEN unique_violation THEN NULL;
    END;
    BEGIN
        INSERT INTO sessions(org_id,owner_principal_id,workspace_id,virtual_user_id) VALUES(1,runtime_principal,'00000000-0000-0000-0000-000000000032',(SELECT virtual_user_id FROM virtual_user_bindings WHERE org_id=2 LIMIT 1));
        RAISE EXCEPTION 'Cross-organization virtual user accepted';
    EXCEPTION WHEN foreign_key_violation THEN NULL;
    END;
    BEGIN
        INSERT INTO agents(org_id,public_id,name,system_prompt,harness_id,virtual_user_id) VALUES(1,'agent_00000000000000000000000000000091','Invalid service','Helpful','00000000-0000-0000-0000-000000000031',default_id);
        RAISE EXCEPTION 'Human account accepted as service';
    EXCEPTION WHEN raise_exception THEN
        IF SQLERRM='Human account accepted as service' THEN RAISE; END IF;
    END;
END;
$$;
