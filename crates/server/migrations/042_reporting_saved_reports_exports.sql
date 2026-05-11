-- Reporting phase 2: saved semantic report definitions.
-- Saved reports persist validated semantic queries, not SQL. Runtime command
-- handlers always inject org scope from the caller context during execution.

CREATE TABLE reporting_saved_reports (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    name TEXT NOT NULL,
    description TEXT,
    query JSONB NOT NULL,
    dashboard JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT reporting_saved_reports_name_not_blank
        CHECK (length(btrim(name)) > 0),
    CONSTRAINT reporting_saved_reports_query_is_object
        CHECK (jsonb_typeof(query) = 'object'),
    CONSTRAINT reporting_saved_reports_dashboard_is_object
        CHECK (dashboard IS NULL OR jsonb_typeof(dashboard) = 'object')
);

CREATE INDEX idx_reporting_saved_reports_org_updated
    ON reporting_saved_reports(org_id, updated_at DESC);

CREATE TRIGGER update_reporting_saved_reports_updated_at
    BEFORE UPDATE ON reporting_saved_reports
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
