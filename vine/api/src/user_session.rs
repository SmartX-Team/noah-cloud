use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{user::UserSpec, user_role::UserRoleSpec};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]

pub struct UserSessionRef {
    #[serde(flatten)]
    pub metadata: UserSessionMetadata,
    pub namespace: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserSessionMetadata {
    pub box_name: Option<String>,
    pub role: UserRoleSpec,
    pub user: UserSpec,
    pub user_name: String,
}
