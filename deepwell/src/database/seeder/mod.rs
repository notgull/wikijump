/*
 * database/seeder/mod.rs
 *
 * DEEPWELL - Wikijump API provider and database manager
 * Copyright (C) 2019-2022 Wikijump Team
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <http://www.gnu.org/licenses/>.
 */

mod data;

use self::data::{SeedData, SitePages};
use crate::api::ApiServerState;
use crate::constants::{ADMIN_USER_ID, SYSTEM_USER_ID};
use crate::services::page::{CreatePage, PageService};
use crate::services::site::{CreateSite, CreateSiteOutput, SiteService};
use crate::services::user::{CreateUser, CreateUserOutput, UpdateUser, UserService};
use crate::services::ServiceContext;
use crate::web::{ProvidedValue, Reference};
use anyhow::Result;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseTransaction, Statement, TransactionTrait,
};

pub async fn seed(state: &ApiServerState) -> Result<()> {
    tide::log::info!("Running seeder...");

    // Set up context
    let txn = state.database.begin().await?;
    let ctx = ServiceContext::from_raw(state, &txn);

    // Ensure seeding has not already been done
    if UserService::exists(&ctx, Reference::from(ADMIN_USER_ID)).await? {
        tide::log::info!("Seeding has already been done");
        return Ok(());
    }

    // Reset sequences so IDs are consistent
    restart_sequence(&txn, "user_user_id_seq").await?;
    restart_sequence(&txn, "page_page_id_seq").await?;
    restart_sequence(&txn, "site_site_id_seq").await?;

    // Load seed data
    tide::log::info!(
        "Loading seed data from {}",
        state.config.seeder_path.display(),
    );

    let SeedData { users, site_pages } = SeedData::load(&state.config.seeder_path)?;

    // Seed user data
    for user in users {
        tide::log::info!("Creating seed user '{}' (ID {})", user.name, user.id);

        // TODO Create user aliases
        let _ = user.aliases;

        let CreateUserOutput { user_id, slug } = UserService::create(
            &ctx,
            CreateUser {
                name: user.name,
                email: user.email,
                password: user.password,
                locale: user.locale,
                is_system: user.is_system,
                is_bot: user.is_bot,
            },
        )
        .await?;

        UserService::update(
            &ctx,
            Reference::Id(user_id),
            UpdateUser {
                email_verified: ProvidedValue::Set(true),
                display_name: ProvidedValue::Set(user.display_name),
                gender: ProvidedValue::Set(user.gender),
                birthday: ProvidedValue::Set(user.birthday),
                biography: ProvidedValue::Set(user.biography),
                user_page: ProvidedValue::Set(user.user_page),
                ..Default::default()
            },
        )
        .await?;

        tide::log::debug!("User created with slug '{}'", slug);
        assert_eq!(user_id, user.id, "Specified user ID doesn't match created");
        assert_eq!(slug, user.slug, "Specified user slug doesn't match created");
    }

    // Seed site data
    for SitePages { site, pages } in site_pages {
        tide::log::info!("Creating seed site '{}' (slug {})", site.name, site.slug);

        let CreateSiteOutput { site_id, slug: _ } = SiteService::create(
            &ctx,
            CreateSite {
                slug: site.slug,
                name: site.name,
                tagline: site.tagline,
                description: site.description,
                locale: site.locale,
            },
        )
        .await?;

        for page in pages {
            tide::log::info!("Creating page '{}' (slug {})", page.title, page.slug);

            PageService::create(
                &ctx,
                site_id,
                CreatePage {
                    wikitext: page.wikitext,
                    title: page.title,
                    alt_title: page.alt_title,
                    slug: page.slug,
                    revision_comments: str!(""),
                    user_id: SYSTEM_USER_ID,
                },
            )
            .await?;
        }
    }

    // After all seeding, modify ID sequences so that they exhibit Wikidot compatibility.
    //
    // This property means that no valid Wikidot ID for a class of object
    // can ever also be a valid Wikijump ID for that same class of object.
    // We do this by putting the start ID for new Wikijump IDs well above
    // what the Wikidot value is likely to reach by the time the project
    // hits production.
    //
    // Some classes of object are not assigned compatibility IDs, either
    // because the ID value does not matter, is unused, or is not exposed.
    //
    // See https://scuttle.atlassian.net/browse/WJ-964

    restart_sequence_with(&txn, "user_user_id_seq", 10000000).await?;
    restart_sequence_with(&txn, "site_site_id_seq", 6000000).await?;
    restart_sequence_with(&txn, "page_page_id_seq", 3000000000).await?;
    restart_sequence_with(&txn, "page_revision_revision_id_seq", 3000000000).await?;

    /*
     * TODO: tables which don't exist yet:
     * restart_sequence_with(&txn, < forum category seq >, 9000000).await?;
     * restart_sequence_with(&txn, < forum thread seq >, 30000000).await?;
     * restart_sequence_with(&txn, < forum post seq >, 7000000).await?;
     */

    txn.commit().await?;
    Ok(())
}

async fn restart_sequence(
    txn: &DatabaseTransaction,
    sequence_name: &'static str,
) -> Result<()> {
    tide::log::debug!("Resetting sequence {sequence_name} (again return to 1)");
    restart_sequence_with(txn, sequence_name, 1).await
}

async fn restart_sequence_with(
    txn: &DatabaseTransaction,
    sequence_name: &'static str,
    new_start_value: i64,
) -> Result<()> {
    tide::log::debug!("Restarting sequence {sequence_name} with {new_start_value}");

    let query = format!("ALTER SEQUENCE {sequence_name} RESTART WITH $1");
    let value: sea_orm::Value = new_start_value.into();

    // SAFETY: We cannot parameterize the sequence name here, so we have to use format!()
    //         However, by requiring that sequence_name be &'static str, we ensure that it
    //         is only applied to hardcoded values and never used for runtime values
    //         (such as ones entered by an external, untrusted user).
    txn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        &query,
        [value],
    ))
    .await?;

    Ok(())
}
