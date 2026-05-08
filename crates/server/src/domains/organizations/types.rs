// Organizations domain types — reuse the public API response DTOs.

pub use crate::api::organizations::OrganizationResponse;
pub use crate::api::resolver::ResolveOrgResponse;
pub type ListOrganizationsResponse = crate::api::common::ListResponse<OrganizationResponse>;
