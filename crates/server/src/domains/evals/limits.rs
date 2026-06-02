// Eval run concurrency and volume limits (EVE-509).
//
// Defaults are set to bound fan-out without throttling legitimate use.
// Operators can tighten or loosen via env without redeploying code.

const DEFAULT_MAX_CONCURRENT_RUNS_PER_ORG: usize = 5;
const DEFAULT_MAX_CASES_PER_RUN: usize = 500;

/// Concurrency and volume limits for eval runs.
#[derive(Clone, Copy, Debug)]
pub struct EvalLimits {
    /// Max eval runs in pending/running state per org simultaneously.
    pub max_concurrent_runs_per_org: usize,
    /// Max number of cases that may be executed in a single eval run.
    pub max_cases_per_run: usize,
}

impl Default for EvalLimits {
    fn default() -> Self {
        Self::from_env()
    }
}

impl EvalLimits {
    pub fn from_env() -> Self {
        Self {
            max_concurrent_runs_per_org: read_usize_env(
                "EVAL_MAX_CONCURRENT_RUNS_PER_ORG",
                DEFAULT_MAX_CONCURRENT_RUNS_PER_ORG,
            ),
            max_cases_per_run: read_usize_env("EVAL_MAX_CASES_PER_RUN", DEFAULT_MAX_CASES_PER_RUN),
        }
    }
}

fn read_usize_env(var: &str, default: usize) -> usize {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(default)
}
