//! `everruns-sandbox-exec`: apply kernel containment, then become the shell.
//!
//! Not a user-facing command. A [`SandboxProvider`] spawns it with the fixed
//! argument list [`WorkerRequest::parse`] accepts, and it never returns on
//! success: it execs bash in place so the restrictions it installed are the
//! ones the shell and every descendant run under.
//!
//! [`SandboxProvider`]: everruns_host::containment::SandboxProvider
//! [`WorkerRequest::parse`]: everruns_host::containment::worker::WorkerRequest::parse

fn main() -> anyhow::Result<()> {
    everruns_host::containment::worker::run_from_args(std::env::args_os().skip(1))?;
    // `run_from_args` returns `Infallible` on success, so this is unreachable.
    Ok(())
}
