//! SeaORM Entity. Generated by sea-orm-codegen 0.10.0

use super::sea_orm_active_enums::UserType;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use time::{Date, OffsetDateTime};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "user")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub user_id: i64,
    pub user_type: UserType,
    pub created_at: OffsetDateTime,
    pub updated_at: Option<OffsetDateTime>,
    pub deleted_at: Option<OffsetDateTime>,
    pub from_wikidot: bool,
    #[sea_orm(column_type = "Text")]
    pub name: String,
    #[sea_orm(column_type = "Text")]
    pub slug: String,
    pub name_changes_left: i16,
    pub last_renamed_at: Option<OffsetDateTime>,
    #[sea_orm(column_type = "Text")]
    pub email: String,
    pub email_verified_at: Option<OffsetDateTime>,
    #[sea_orm(column_type = "Text")]
    pub password: String,
    #[sea_orm(column_type = "Text", nullable)]
    pub multi_factor_secret: Option<String>,
    pub multi_factor_recovery_codes: Option<Vec<String>>,
    #[sea_orm(column_type = "Text")]
    pub locale: String,
    pub avatar_s3_hash: Option<Vec<u8>>,
    #[sea_orm(column_type = "Text", nullable)]
    pub real_name: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub gender: Option<String>,
    pub birthday: Option<Date>,
    #[sea_orm(column_type = "Text", nullable)]
    pub location: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub biography: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub user_page: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::session::Entity")]
    Session,
    #[sea_orm(has_many = "super::page_revision::Entity")]
    PageRevision,
    #[sea_orm(has_many = "super::page_attribution::Entity")]
    PageAttribution,
    #[sea_orm(has_many = "super::page_lock::Entity")]
    PageLock,
    #[sea_orm(has_many = "super::file_revision::Entity")]
    FileRevision,
}

impl Related<super::session::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl Related<super::page_revision::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::PageRevision.def()
    }
}

impl Related<super::page_attribution::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::PageAttribution.def()
    }
}

impl Related<super::page_lock::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::PageLock.def()
    }
}

impl Related<super::file_revision::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::FileRevision.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
