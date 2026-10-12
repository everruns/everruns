-- Organization web search defaults: the provider agents set to "Organization
-- default" use, the Automatic order, whether model provider search is allowed,
-- the fallback mode, results per search and blocked domains. NULL means the
-- built-in defaults (Automatic, native allowed, fall back, 8 results). The
-- shape is validated by the API (`WebSearchDefaults` in everruns-contracts).
-- See knowledge/integrations/web-search.md.
ALTER TABLE organization_settings
ADD COLUMN web_search_defaults JSONB;
