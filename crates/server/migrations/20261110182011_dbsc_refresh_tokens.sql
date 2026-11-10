-- Device Bound Session Credentials (crates/server/src/auth/dbsc.rs). A bound
-- refresh token is the browser's DBSC session: it stores the device's public
-- key and the outstanding refresh challenge. NULL dbsc_id means unbound;
-- `/v1/auth/refresh` only consumes unbound rows.
ALTER TABLE public.refresh_tokens
    ADD COLUMN dbsc_id text,
    ADD COLUMN dbsc_alg text,
    ADD COLUMN dbsc_public_jwk jsonb,
    ADD COLUMN dbsc_bound_at timestamp with time zone,
    ADD COLUMN dbsc_challenge text,
    ADD COLUMN dbsc_challenge_expires_at timestamp with time zone;

CREATE UNIQUE INDEX refresh_tokens_dbsc_id_key ON public.refresh_tokens (dbsc_id)
    WHERE dbsc_id IS NOT NULL;
