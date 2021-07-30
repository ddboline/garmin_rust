use derive_more::{Deref, Display, From, FromStr, Into};
use rweb::openapi::{Entity, Schema, Type};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(
    Serialize,
    Debug,
    FromStr,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Clone,
    Copy,
    Deref,
    Into,
    From,
    Deserialize,
    Hash,
    Display,
)]
pub struct UuidWrapper(Uuid);

impl Entity for UuidWrapper {
    #[inline]
    fn describe() -> Schema {
        Schema {
            schema_type: Some(Type::String),
            format: "datetime".into(),
            ..Schema::default()
        }
    }
}
