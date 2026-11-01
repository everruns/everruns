-- usage_policies_and_costs
-- Budgets become usage policies and every usage gets a cost the org can
-- explain. See knowledge/security/budgeting.md.
--
-- 1. `budgets.policy` carries the policy's name, steps (notify, ask to
--    continue, stop) and optional filter. Empty means the legacy shape:
--    soft_limit asks to continue, the limit stops.
ALTER TABLE public.budgets ADD COLUMN policy jsonb DEFAULT '{}'::jsonb NOT NULL;

-- 2. The `credits` budget currency (1 credit = 1,000 tokens) is retired: the
--    word belongs to the hosted platform's balance. Rows convert to tokens at
--    the rate they were metered at; their ledger history stays as written.
UPDATE public.budgets
SET currency = 'tokens',
    "limit" = "limit" * 1000,
    soft_limit = soft_limit * 1000,
    balance = balance * 1000,
    metadata = COALESCE(metadata, '{}'::jsonb) || '{"converted_from":"credits"}'::jsonb
WHERE currency = 'credits';

-- 3. Every ledger posting says where its amount came from. Cost postings
--    (budget_id NULL, currency usd) carry one of: host, rate, provider,
--    list_price, unpriced. Budget debits copy the basis of the cost they
--    debit. Adding a column does not touch existing rows, so the append-only
--    trigger is unaffected.
ALTER TABLE public.usage_ledger ADD COLUMN basis text;
ALTER TABLE public.usage_ledger ADD COLUMN rate_id uuid;

-- One cost posting per journal row and kind (`cost` the first rating,
-- `cost_host` the correction a host makes when it reports what it charged).
CREATE UNIQUE INDEX idx_usage_ledger_cost_posting
    ON public.usage_ledger USING btree (journal_id, meter_source)
    WHERE (budget_id IS NULL);
-- A policy is debited once per usage and kind: the first rating, then at
-- most one host correction (`host_correction`). The old index keyed only on
-- (journal, budget, currency) and would refuse the correction.
DROP INDEX public.idx_usage_ledger_journal_budget_currency;
CREATE UNIQUE INDEX idx_usage_ledger_journal_budget_source
    ON public.usage_ledger USING btree (journal_id, budget_id, currency, meter_source)
    WHERE (budget_id IS NOT NULL);
CREATE INDEX idx_usage_ledger_org_costs
    ON public.usage_ledger USING btree (org_id, created_at DESC)
    WHERE (budget_id IS NULL);

-- 4. The org's own price list. Rows are never edited: a change is a new row
--    with a later `effective_from`; retiring a rate is a row with
--    `retired = true`. The most specific match wins, then the latest start.
CREATE TABLE public.cost_rates (
    id uuid DEFAULT public.uuidv7() NOT NULL PRIMARY KEY,
    org_id bigint NOT NULL REFERENCES public.organizations(org_id),
    meter text NOT NULL,
    match jsonb DEFAULT '{}'::jsonb NOT NULL,
    unit_price_usd double precision,
    list_price_multiplier double precision,
    unit text NOT NULL,
    effective_from timestamp with time zone NOT NULL,
    retired boolean DEFAULT false NOT NULL,
    reason text,
    created_by uuid,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT cost_rates_meter_check CHECK (meter = ANY (ARRAY['model.tokens'::text, 'sandbox.time'::text])),
    CONSTRAINT cost_rates_price_check CHECK (
        retired
        OR (unit_price_usd IS NOT NULL AND list_price_multiplier IS NULL AND unit_price_usd >= 0)
        OR (unit_price_usd IS NULL AND list_price_multiplier IS NOT NULL AND list_price_multiplier >= 0)
    )
);
CREATE INDEX idx_cost_rates_org_meter ON public.cost_rates USING btree (org_id, meter, effective_from DESC);

CREATE TRIGGER prevent_cost_rate_mutation BEFORE DELETE OR UPDATE ON public.cost_rates
    FOR EACH ROW EXECUTE FUNCTION public.prevent_usage_record_mutation();

-- 5. "Ask to continue" answers. A person lets one session go past a policy's
--    ask step for the policy's current window.
CREATE TABLE public.usage_policy_approvals (
    id uuid DEFAULT public.uuidv7() NOT NULL PRIMARY KEY,
    org_id bigint NOT NULL REFERENCES public.organizations(org_id),
    budget_id uuid NOT NULL REFERENCES public.budgets(id),
    session_id uuid NOT NULL,
    approved_by uuid,
    window_started_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);
CREATE INDEX idx_usage_policy_approvals_lookup
    ON public.usage_policy_approvals USING btree (budget_id, session_id, created_at DESC);

-- 6. Notify steps fire once per policy window.
CREATE TABLE public.usage_policy_notices (
    budget_id uuid NOT NULL REFERENCES public.budgets(id),
    step_at double precision NOT NULL,
    window_started_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    PRIMARY KEY (budget_id, step_at, window_started_at)
);

-- 7. Calendar windows (day, week, month) reset at midnight in the org's
--    timezone. NULL means UTC, which is what every window used before.
ALTER TABLE public.organization_settings ADD COLUMN timezone text;
