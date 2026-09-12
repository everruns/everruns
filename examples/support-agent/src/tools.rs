use everruns::capability::serde_json::{self, Value};

fn customer(id: &str) -> Result<Value, String> {
    let customers: Value =
        serde_json::from_str(include_str!("customers.json")).map_err(|error| error.to_string())?;
    customers
        .get(id)
        .cloned()
        .ok_or_else(|| format!("Unknown demo customer: {id}"))
}

#[everruns::tool]
/// Return factual account state, without prescribing a resolution.
pub async fn lookup_customer(customer_id: String) -> Result<String, String> {
    customer(&customer_id).map(|value| value.to_string())
}

#[everruns::tool]
/// Read the service's account-recovery rules before recommending a next step.
pub async fn read_support_policy() -> Result<String, String> {
    Ok(include_str!("policy.md").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_facts_distinguish_mfa_lockout_and_browser_cases() {
        let mfa = customer("cust_mfa").unwrap();
        assert_eq!(mfa["mfa_enabled"], true);
        assert_eq!(mfa["authenticator_available"], false);
        assert_eq!(mfa["recovery_codes_available"], false);
        assert_eq!(customer("cust_locked").unwrap()["lockout_minutes"], 15);
        let browser = customer("cust_browser").unwrap();
        assert_eq!(browser["mfa_enabled"], false);
        assert_eq!(browser["lockout_minutes"], 0);
    }
    #[test]
    fn unknown_customer_is_not_silently_replaced() {
        assert!(customer("../customers.json").is_err());
        assert!(customer("not-a-customer").is_err());
    }
}
