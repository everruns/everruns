// Users domain types — reuse the public API user DTO.

pub use crate::api::users::User;
pub type ListUsersResponse = crate::api::common::ListResponse<User>;
