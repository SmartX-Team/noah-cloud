use ark_core_k8s::data::Url;
use serde::{Deserialize, Serialize};

use super::product::ProductSpec;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubSpec {
    pub cost: <Self as super::BaseModel>::Cost,
    pub endpoint: Url,
}

impl super::BaseModel for SubSpec {
    type Id = <ProductSpec as super::BaseModel>::Id;
    type Cost = <ProductSpec as super::BaseModel>::Cost;
    type Count = <ProductSpec as super::BaseModel>::Count;
}

impl super::BaseModelItem for SubSpec {
    fn cost(&self) -> <Self as super::BaseModel>::Cost {
        self.cost
    }
}
