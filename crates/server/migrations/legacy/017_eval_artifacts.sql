-- Add artifact specs to eval cases and collected artifact payloads to eval case results.

ALTER TABLE eval_cases
    ADD COLUMN artifacts JSONB;

ALTER TABLE eval_case_results
    ADD COLUMN artifacts JSONB;
