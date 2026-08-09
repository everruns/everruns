use everruns::{Agent, CapabilityRef, CapabilitySpec, IntoCapability, Model};

struct VendorCapability {
    region: String,
}

impl IntoCapability for VendorCapability {
    fn into_capability(self) -> CapabilitySpec {
        CapabilityRef::new("vendor.search")
            .config(everruns::capability::serde_json::json!({ "region": self.region }))
            .into()
    }
}

fn main() {
    let _agent = Agent::builder()
        .instructions("Use vendor search when available.")
        .model(Model::simulated("done"))
        .capability(VendorCapability {
            region: "us-central".into(),
        })
        .build()
        .unwrap();
}
