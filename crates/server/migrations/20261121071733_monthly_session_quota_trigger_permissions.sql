-- THREAT[TM-DOS-024]: Restricted session writers consume quota through the
-- installed trigger without receiving direct access to settings or counters.
-- Resolve the existing public tables before any caller-controlled temp tables.
ALTER FUNCTION public.count_monthly_session_creation()
    SECURITY DEFINER SET search_path TO pg_catalog, public, pg_temp;

-- Only the existing sessions trigger needs to call this function.
REVOKE EXECUTE ON FUNCTION public.count_monthly_session_creation() FROM PUBLIC;
