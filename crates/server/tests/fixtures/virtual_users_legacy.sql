-- Pre-cutover fixture: one membership, multiple memberships, external speaker,
-- archived starter, pin, service account, encrypted grants and a pending lease.
INSERT INTO organizations(org_id,public_id,name) VALUES (2,'org_00000000000000000000000000000002','Second');
INSERT INTO users(id,email,name) VALUES
 ('00000000-0000-0000-0000-000000000011','single@example.com','Single'),
 ('00000000-0000-0000-0000-000000000012','multiple@example.com','Multiple');
INSERT INTO organization_members(org_id,user_id,role) VALUES
 (1,'00000000-0000-0000-0000-000000000011','owner'),
 (1,'00000000-0000-0000-0000-000000000012','member'),
 (2,'00000000-0000-0000-0000-000000000012','member');
INSERT INTO principals(id,public_id,org_id,kind,subject_id,resolved_user_id) VALUES
 ('00000000-0000-0000-0000-000000000021','principal_00000000000000000000000000000021',1,'user','00000000-0000-0000-0000-000000000011','00000000-0000-0000-0000-000000000011');
INSERT INTO principals(id,public_id,org_id,kind,subject_id,metadata) VALUES
 ('00000000-0000-0000-0000-000000000022','principal_00000000000000000000000000000022',1,'user','00000000-0000-0000-0000-000000000013','{"source":"external_actor","name":"Slack speaker","external_source":"slack","external_actor_id":"U123","external_actor_metadata":{"team_id":"T123"}}');
INSERT INTO harnesses(id,org_id,name,system_prompt) VALUES
 ('00000000-0000-0000-0000-000000000031',1,'platform-chat','Helpful');
INSERT INTO workspaces(id,org_id,public_id,name) VALUES
 ('00000000-0000-0000-0000-000000000032',1,'wsp_00000000000000000000000000000032','Workspace');
INSERT INTO sessions(id,org_id,harness_id,workspace_id,owner_principal_id,resolved_owner_user_id,title,source,tags,archived_at) VALUES
 ('00000000-0000-0000-0000-000000000041',1,'00000000-0000-0000-0000-000000000031','00000000-0000-0000-0000-000000000032','00000000-0000-0000-0000-000000000021','00000000-0000-0000-0000-000000000011','Platform Chat','chat',ARRAY['chat','platform-chat-starter'],now());
INSERT INTO session_participants(id,org_id,session_id,kind,principal_id,role) VALUES
 ('00000000-0000-0000-0000-000000000051',1,'00000000-0000-0000-0000-000000000041','user','00000000-0000-0000-0000-000000000021','member');
INSERT INTO pinned_sessions(user_id,session_id,org_id) VALUES
 ('00000000-0000-0000-0000-000000000011','00000000-0000-0000-0000-000000000041',1);
INSERT INTO agent_identities(id,org_id,name) VALUES
 ('00000000-0000-0000-0000-000000000061',1,'Service');
INSERT INTO agent_identity_connections(id,agent_identity_id,provider,connection_type,access_token_encrypted) VALUES
 ('00000000-0000-0000-0000-000000000071','00000000-0000-0000-0000-000000000061','github','api_key',decode('abcd','hex'));
INSERT INTO user_connections(id,user_id,provider,connection_type,access_token_encrypted) VALUES
 ('00000000-0000-0000-0000-000000000072','00000000-0000-0000-0000-000000000011','daytona','api_key',decode('1234','hex')),
 ('00000000-0000-0000-0000-000000000073','00000000-0000-0000-0000-000000000012','daytona','api_key',decode('5678','hex'));
INSERT INTO leased_resources(id,public_id,org_id,session_id,provider,resource_type,external_id,status,owner_user_id,lease_duration_seconds,last_touched_at,lease_expires_at) VALUES
 ('00000000-0000-0000-0000-000000000081','resource_00000000000000000000000000000081',1,'00000000-0000-0000-0000-000000000041','daytona','sandbox','fixture-sandbox','active','00000000-0000-0000-0000-000000000012',3600,now(),now()+interval '1 hour');

INSERT INTO github_apps(id,org_id,agent_identity_id,app_id,slug,name,html_url,private_key_encrypted) VALUES
 ('00000000-0000-0000-0000-000000000091',1,'00000000-0000-0000-0000-000000000061',9001,'fixture','Fixture','https://github.com/apps/fixture',decode('cafe','hex'));
INSERT INTO payment_accounts(id,org_id,label,owner_type,owner_id,rail,credential_encrypted,status) VALUES
 ('00000000-0000-0000-0000-000000000092',1,'Fixture','agent_identity','identity_00000000000000000000000000000061','x402_base',decode('beef','hex'),'active');
INSERT INTO payment_policies(org_id,payment_account_id,subject_type,subject_id,status) VALUES
 (1,'00000000-0000-0000-0000-000000000092','agent_identity','identity_00000000000000000000000000000061','active');
