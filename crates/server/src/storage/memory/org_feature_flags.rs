// In-memory storage: per-organization feature flag opt-ins

use super::InMemoryDatabase;
use anyhow::Result;
use std::collections::HashMap;

impl InMemoryDatabase {
    pub async fn list_org_feature_flags(&self, org_id: i64) -> Result<HashMap<String, bool>> {
        Ok(self
            .org_feature_flags
            .read()
            .get(&org_id)
            .cloned()
            .unwrap_or_default())
    }

    pub async fn replace_org_feature_flags(
        &self,
        org_id: i64,
        flags: &HashMap<String, bool>,
    ) -> Result<()> {
        let mut store = self.org_feature_flags.write();
        let entry = store.entry(org_id).or_default();
        for (flag_name, enabled) in flags {
            if *enabled {
                entry.insert(flag_name.clone(), true);
            } else {
                entry.remove(flag_name);
            }
        }
        Ok(())
    }
}
