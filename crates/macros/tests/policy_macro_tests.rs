// Integration tests for the #[policy] proc macro.
//
// These tests verify that the macro correctly injects policy evaluation
// into function bodies. Uses a minimal Policy/Caller implementation
// to test macro behavior independently of the full permissions system.

use std::fmt;

/// Minimal Caller type for testing the macro.
#[derive(Debug)]
struct Caller {
    allowed: bool,
}

/// Minimal error type for testing.
#[derive(Debug)]
struct PolicyError(String);

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PolicyError: {}", self.0)
    }
}

impl std::error::Error for PolicyError {}

/// Minimal Policy type for testing the macro.
struct Policy {
    id: &'static str,
}

impl Policy {
    fn evaluate(&self, caller: &Caller) -> Result<(), PolicyError> {
        if caller.allowed {
            Ok(())
        } else {
            Err(PolicyError(format!("denied by {}", self.id)))
        }
    }
}

const TEST_POLICY: Policy = Policy { id: "test.policy" };
const STRICT_POLICY: Policy = Policy {
    id: "strict.policy",
};

// -- Tests --

struct MyService;

impl MyService {
    #[everruns_macros::policy(TEST_POLICY)]
    async fn do_work(&self, caller: &Caller) -> Result<String, PolicyError> {
        Ok("work done".to_string())
    }

    #[everruns_macros::policy(TEST_POLICY)]
    async fn do_work_with_args(&self, caller: &Caller, name: &str) -> Result<String, PolicyError> {
        Ok(format!("hello {}", name))
    }

    #[everruns_macros::policy(TEST_POLICY)]
    #[everruns_macros::policy(STRICT_POLICY)]
    async fn do_restricted(&self, caller: &Caller) -> Result<String, PolicyError> {
        Ok("restricted work".to_string())
    }
}

// Free function (not a method)
#[everruns_macros::policy(TEST_POLICY)]
async fn free_function(caller: &Caller, value: i32) -> Result<i32, PolicyError> {
    Ok(value * 2)
}

#[tokio::test]
async fn policy_allows_when_caller_permitted() {
    let svc = MyService;
    let caller = Caller { allowed: true };
    let result = svc.do_work(&caller).await;
    assert_eq!(result.unwrap(), "work done");
}

#[tokio::test]
async fn policy_denies_when_caller_not_permitted() {
    let svc = MyService;
    let caller = Caller { allowed: false };
    let result = svc.do_work(&caller).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().0.contains("denied by test.policy"));
}

#[tokio::test]
async fn policy_passes_args_through() {
    let svc = MyService;
    let caller = Caller { allowed: true };
    let result = svc.do_work_with_args(&caller, "world").await;
    assert_eq!(result.unwrap(), "hello world");
}

#[tokio::test]
async fn stacked_policies_both_checked() {
    let svc = MyService;
    let allowed = Caller { allowed: true };
    let denied = Caller { allowed: false };

    // Both policies pass
    assert!(svc.do_restricted(&allowed).await.is_ok());

    // Both policies fail (outermost runs first)
    let err = svc.do_restricted(&denied).await.unwrap_err();
    assert!(
        err.0.contains("denied by"),
        "Expected 'denied by' in: {}",
        err.0
    );
}

#[tokio::test]
async fn policy_works_on_free_functions() {
    let caller = Caller { allowed: true };
    let result = free_function(&caller, 21).await;
    assert_eq!(result.unwrap(), 42);

    let denied = Caller { allowed: false };
    assert!(free_function(&denied, 21).await.is_err());
}
