// Organizations domain types — reuse the public API response DTOs.

pub use crate::api::organizations::OrganizationResponse;
pub type ListOrganizationsResponse = crate::api::common::ListResponse<OrganizationResponse>;
