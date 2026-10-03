//! Scoped deployment-grade override for domain tests.

/// `allow_local_urls` is honored only under `DEPLOYMENT_GRADE=dev`
/// (TM-AGENT-024), and the A2A provider reads the grade from the process
/// environment. Scope the dev grade to each test that uses it so the rest of the binary keeps
/// its own grade; CI runs this binary with `--test-threads=1`.
pub(crate) struct DevGradeGuard(Option<std::ffi::OsString>);

impl DevGradeGuard {
    pub(crate) fn set() -> Self {
        let previous = std::env::var_os("DEPLOYMENT_GRADE");
        // SAFETY: tests in this binary that touch the environment run serially.
        unsafe { std::env::set_var("DEPLOYMENT_GRADE", "dev") };
        Self(previous)
    }
}

impl Drop for DevGradeGuard {
    fn drop(&mut self) {
        // SAFETY: see `DevGradeGuard::set`.
        unsafe {
            match self.0.take() {
                Some(value) => std::env::set_var("DEPLOYMENT_GRADE", value),
                None => std::env::remove_var("DEPLOYMENT_GRADE"),
            }
        }
    }
}
