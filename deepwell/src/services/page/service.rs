/*
 * services/page/service.rs
 *
 * DEEPWELL - Wikijump API provider and database manager
 * Copyright (C) 2019-2023 Wikijump Team
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

use super::prelude::*;
use crate::models::page::{self, Entity as Page, Model as PageModel};
use crate::models::page_category::Model as PageCategoryModel;
use crate::services::filter::{FilterClass, FilterType};
use crate::services::page_revision::{
    CreateFirstPageRevision, CreateFirstPageRevisionOutput, CreatePageRevision,
    CreatePageRevisionBody, CreatePageRevisionOutput, CreateResurrectionPageRevision,
    CreateTombstonePageRevision,
};
use crate::services::{CategoryService, FilterService, PageRevisionService, TextService};
use crate::utils::{get_category_name, trim_default};
use crate::web::PageOrder;
use wikidot_normalize::normalize;

#[derive(Debug)]
pub struct PageService;

impl PageService {
    pub async fn create(
        ctx: &ServiceContext<'_>,
        CreatePage {
            site_id,
            wikitext,
            title,
            alt_title,
            mut slug,
            revision_comments: comments,
            user_id,
            bypass_filter,
        }: CreatePage,
    ) -> Result<CreatePageOutput> {
        let txn = ctx.transaction();

        // Ensure row consistency
        normalize(&mut slug);
        Self::check_conflicts(ctx, site_id, &slug, "create").await?;

        // Perform filter validation
        if !bypass_filter {
            Self::run_filter(
                ctx,
                site_id,
                Some(&wikitext),
                Some(&title),
                alt_title.as_ref(),
            )
            .await?;
        }

        // Create category if not already present
        let PageCategoryModel { category_id, .. } =
            CategoryService::get_or_create(ctx, site_id, get_category_name(&slug))
                .await?;

        // Insert page
        let model = page::ActiveModel {
            site_id: Set(site_id),
            page_category_id: Set(category_id),
            slug: Set(slug.clone()),
            ..Default::default()
        };
        let page = model.insert(txn).await?;

        // Commit first revision
        let revision_input = CreateFirstPageRevision {
            user_id,
            comments,
            wikitext,
            title,
            alt_title,
            slug: slug.clone(),
        };

        let CreateFirstPageRevisionOutput {
            revision_id,
            parser_errors,
        } = PageRevisionService::create_first(ctx, site_id, page.page_id, revision_input)
            .await?;

        // Build and return
        Ok(CreatePageOutput {
            page_id: page.page_id,
            slug,
            revision_id,
            parser_errors,
        })
    }

    pub async fn edit(
        ctx: &ServiceContext<'_>,
        EditPage {
            site_id,
            page: reference,
            revision_comments: comments,
            user_id,
            body:
                EditPageBody {
                    wikitext,
                    title,
                    alt_title,
                    tags,
                },
        }: EditPage<'_>,
    ) -> Result<Option<EditPageOutput>> {
        let txn = ctx.transaction();
        let PageModel { page_id, .. } = Self::get(ctx, site_id, reference).await?;

        // Perform filter validation
        Self::run_filter(
            ctx,
            site_id,
            wikitext.to_option(),
            title.to_option(),
            // Flatten what is essentially Option<Option<_>>
            match alt_title {
                ProvidedValue::Set(Some(ref alt_title)) => Some(alt_title),
                _ => None,
            },
        )
        .await?;

        // Get latest revision
        let last_revision =
            PageRevisionService::get_latest(ctx, site_id, page_id).await?;

        // Create new revision
        //
        // A response of None means no revision was created
        // because none of the data actually changed.

        let revision_input = CreatePageRevision {
            user_id,
            comments,
            body: CreatePageRevisionBody {
                wikitext,
                title,
                alt_title,
                tags,
                ..Default::default()
            },
        };

        let revision_output = PageRevisionService::create(
            ctx,
            site_id,
            page_id,
            revision_input,
            last_revision,
        )
        .await?;

        // Set page updated_at column.
        //
        // Previously this was conditional on whether a revision was actually created.
        // But since this rerenders regardless, we need to update the page row.
        let model = page::ActiveModel {
            page_id: Set(page_id),
            updated_at: Set(Some(now())),
            ..Default::default()
        };

        model.update(txn).await?;

        // Build and return
        Ok(revision_output)
    }

    /// Moves a page from from one slug to another.
    pub async fn r#move(
        ctx: &ServiceContext<'_>,
        MovePage {
            site_id,
            page: reference,
            mut new_slug,
            revision_comments: comments,
            user_id,
        }: MovePage<'_>,
    ) -> Result<MovePageOutput> {
        let txn = ctx.transaction();

        let PageModel {
            page_id,
            slug: old_slug,
            ..
        } = Self::get(ctx, site_id, reference).await?;

        // Check that a move is actually taking place,
        // and that a page with that slug doesn't already exist.
        normalize(&mut new_slug);
        if old_slug == new_slug {
            tide::log::error!("Source and destination slugs are the same: {}", old_slug);
            return Err(Error::BadRequest);
        }

        Self::check_conflicts(ctx, site_id, &new_slug, "move").await?;

        // Create category if not already present
        let PageCategoryModel { category_id, .. } =
            CategoryService::get_or_create(ctx, site_id, get_category_name(&new_slug))
                .await?;

        // Get latest revision
        let last_revision =
            PageRevisionService::get_latest(ctx, site_id, page_id).await?;

        // Create revision for move
        let revision_input = CreatePageRevision {
            user_id,
            comments,
            body: CreatePageRevisionBody {
                slug: ProvidedValue::Set(new_slug.clone()),
                ..Default::default()
            },
        };

        let revision_output = PageRevisionService::create(
            ctx,
            site_id,
            page_id,
            revision_input,
            last_revision,
        )
        .await?;

        // Update page after move. This changes:
        // * slug             -- New slug for the page
        // * page_category_id -- In case the category also changed
        // * updated_at       -- This is updated every time a page is changed
        let model = page::ActiveModel {
            page_id: Set(page_id),
            slug: Set(new_slug.clone()),
            page_category_id: Set(category_id),
            updated_at: Set(Some(now())),
            ..Default::default()
        };

        model.update(txn).await?;

        // Build and return

        match revision_output {
            Some(CreatePageRevisionOutput {
                revision_id,
                revision_number,
                parser_errors,
            }) => Ok(MovePageOutput {
                old_slug,
                new_slug,
                revision_id,
                revision_number,
                parser_errors,
            }),
            None => {
                tide::log::error!("Page move did not create new revision");
                Err(Error::BadRequest)
            }
        }
    }

    pub async fn delete(
        ctx: &ServiceContext<'_>,
        DeletePage {
            site_id,
            page: reference,
            user_id,
            revision_comments: comments,
        }: DeletePage<'_>,
    ) -> Result<DeletePageOutput> {
        let txn = ctx.transaction();
        let PageModel { page_id, .. } = Self::get(ctx, site_id, reference).await?;

        // Get latest revision
        let last_revision =
            PageRevisionService::get_latest(ctx, site_id, page_id).await?;

        // Create tombstone revision
        // This also updates backlinks, includes, etc
        let output = PageRevisionService::create_tombstone(
            ctx,
            CreateTombstonePageRevision {
                site_id,
                page_id,
                user_id,
                comments,
            },
            last_revision,
        )
        .await?;

        // Set deletion flag
        let model = page::ActiveModel {
            page_id: Set(page_id),
            deleted_at: Set(Some(now())),
            ..Default::default()
        };

        // Update and return
        model.update(txn).await?;
        Ok((output, page_id).into())
    }

    /// Restore a deleted page, causing it to be undeleted.
    pub async fn restore(
        ctx: &ServiceContext<'_>,
        RestorePage {
            site_id,
            page_id,
            user_id,
            slug,
            revision_comments: comments,
        }: RestorePage,
    ) -> Result<RestorePageOutput> {
        let txn = ctx.transaction();
        let page = Self::get_direct(ctx, page_id).await?;
        let slug = slug.unwrap_or(page.slug);

        // Do page checks:
        // - Site is correct
        // - Page is deleted
        // - Slug doesn't already exist

        if page.site_id != site_id {
            tide::log::warn!("Page's site ID and passed site ID do not match");
            return Err(Error::NotFound);
        }

        if page.deleted_at.is_none() {
            tide::log::warn!("Page requested to be restored is not currently deleted");
            return Err(Error::BadRequest);
        }

        Self::check_conflicts(ctx, site_id, &slug, "restore").await?;

        // Create category if not already present
        let category =
            CategoryService::get_or_create(ctx, site_id, get_category_name(&slug))
                .await?;

        // Get latest revision
        let last_revision =
            PageRevisionService::get_latest(ctx, site_id, page_id).await?;

        // Create resurrection revision
        // This also updates backlinks, includes, etc.
        let output = PageRevisionService::create_resurrection(
            ctx,
            CreateResurrectionPageRevision {
                site_id,
                page_id,
                user_id,
                comments,
                new_slug: slug.clone(),
            },
            last_revision,
        )
        .await?;

        // Set deletion flag
        let model = page::ActiveModel {
            page_id: Set(page_id),
            page_category_id: Set(category.category_id),
            deleted_at: Set(None),
            ..Default::default()
        };

        // Update and return
        model.update(txn).await?;
        Ok((output, slug).into())
    }

    /// Rolls back a page to be the same as it was in a previous revision.
    /// Also called "page reset".
    ///
    /// It changes the page to have the exact state it had in a previous
    /// revision, regardless of any changes since.
    ///
    /// This is equivalent to Wikidot's concept of a "revert".
    pub async fn rollback(
        ctx: &ServiceContext<'_>,
        RollbackPage {
            site_id,
            page: reference,
            revision_number,
            revision_comments: comments,
            user_id,
        }: RollbackPage<'_>,
    ) -> Result<Option<EditPageOutput>> {
        let txn = ctx.transaction();
        let PageModel { page_id, .. } = Self::get(ctx, site_id, reference).await?;

        // Get target revision and latest revision
        let (target_revision, last_revision) = try_join!(
            PageRevisionService::get(ctx, site_id, page_id, revision_number),
            PageRevisionService::get_latest(ctx, site_id, page_id),
        )?;

        // Note: we can't just copy the wikitext_hash because we
        //       need its actual value for rendering.
        //       This isn't run here, but in PageRevisionService::create().
        let wikitext = TextService::get(ctx, &target_revision.wikitext_hash).await?;

        // Create new revision
        //
        // Copy the body of the target revision

        let revision_input = CreatePageRevision {
            user_id,
            comments,
            body: CreatePageRevisionBody {
                wikitext: ProvidedValue::Set(wikitext),
                title: ProvidedValue::Set(target_revision.title),
                alt_title: ProvidedValue::Set(target_revision.alt_title),
                tags: ProvidedValue::Set(target_revision.tags),
                slug: ProvidedValue::Unset, // rollbacks should never move a page
            },
        };

        let revision_output = PageRevisionService::create(
            ctx,
            site_id,
            page_id,
            revision_input,
            last_revision,
        )
        .await?;

        // Set page updated_at column.
        let model = page::ActiveModel {
            page_id: Set(page_id),
            updated_at: Set(Some(now())),
            ..Default::default()
        };

        model.update(txn).await?;

        // Build and return
        Ok(revision_output)
    }

    /// Undoes a past revision, applying the inverse of its changes.
    ///
    /// It looks at the changes made in that revision, and does the
    /// inverse there specifically. It is contextual, and preserves
    /// all other changes made since.
    ///
    /// However, this can cause it to conflict, which will occur if
    /// the reversed changes interfere with other changes made since.
    ///
    /// This is equivalent to git's concept of a "revert".
    #[allow(dead_code)]
    pub async fn undo(
        _ctx: &ServiceContext<'_>,
        _site_id: i64,
        _page_id: i64,
        _revision_number: i32,
    ) -> Result<EditPageOutput> {
        todo!()
    }

    #[inline]
    pub async fn get(
        ctx: &ServiceContext<'_>,
        site_id: i64,
        reference: Reference<'_>,
    ) -> Result<PageModel> {
        find_or_error(Self::get_optional(ctx, site_id, reference)).await
    }

    pub async fn get_optional(
        ctx: &ServiceContext<'_>,
        site_id: i64,
        reference: Reference<'_>,
    ) -> Result<Option<PageModel>> {
        let txn = ctx.transaction();
        let page = {
            let condition = match reference {
                Reference::Id(id) => page::Column::PageId.eq(id),
                Reference::Slug(slug) => {
                    // Trim off _default category if present
                    page::Column::Slug.eq(trim_default(&slug))
                }
            };

            Page::find()
                .filter(
                    Condition::all()
                        .add(condition)
                        .add(page::Column::SiteId.eq(site_id))
                        .add(page::Column::DeletedAt.is_null()),
                )
                .one(txn)
                .await?
        };

        Ok(page)
    }

    /// Gets the page ID from a reference, looking up if necessary.
    ///
    /// Convenience method since this is much more common than the optional
    /// case, and we don't want to perform a redundant check for site existence
    /// later as part of the actual query.
    pub async fn get_id(
        ctx: &ServiceContext<'_>,
        site_id: i64,
        reference: Reference<'_>,
    ) -> Result<i64> {
        match reference {
            Reference::Id(page_id) => Ok(page_id),
            Reference::Slug(slug) => {
                // For slugs we pass-through the call so that slug-handling is done consistently.
                let PageModel { page_id, .. } =
                    Self::get(ctx, site_id, Reference::Slug(slug)).await?;

                Ok(page_id)
            }
        }
    }

    #[inline]
    pub async fn get_direct(ctx: &ServiceContext<'_>, page_id: i64) -> Result<PageModel> {
        find_or_error(Self::get_direct_optional(ctx, page_id)).await
    }

    pub async fn get_direct_optional(
        ctx: &ServiceContext<'_>,
        page_id: i64,
    ) -> Result<Option<PageModel>> {
        let txn = ctx.transaction();
        let page = Page::find_by_id(page_id).one(txn).await?;
        Ok(page)
    }

    /// Get all pages in a site, with potential conditions.
    ///
    /// The `category` argument:
    /// * If it is `Some(_)`, then it specifies a reference to the category
    ///   to select from.
    /// * If it is `None`, then all pages on the site are selected.
    ///
    /// The `deleted` argument:
    /// * If it is `Some(true)`, then it only returns pages which have been deleted.
    /// * If it is `Some(false)`, then it only returns pages which are extant.
    /// * If it is `None`, then it returns all pages regardless of deletion status.
    ///
    /// For the `order` argument, see documentation on `PageOrder`.
    // TODO add pagination
    pub async fn get_all(
        ctx: &ServiceContext<'_>,
        site_id: i64,
        category: Option<Reference<'_>>,
        deleted: Option<bool>,
        order: PageOrder,
    ) -> Result<Vec<PageModel>> {
        let txn = ctx.transaction();

        let category_condition = match category {
            None => None,
            Some(category_reference) => {
                let PageCategoryModel { category_id, .. } =
                    CategoryService::get(ctx, site_id, category_reference).await?;

                Some(page::Column::PageCategoryId.eq(category_id))
            }
        };

        let deleted_condition = match deleted {
            Some(true) => Some(page::Column::DeletedAt.is_not_null()),
            Some(false) => Some(page::Column::DeletedAt.is_null()),
            None => None,
        };

        let pages = Page::find()
            .filter(
                Condition::all()
                    .add(page::Column::SiteId.eq(site_id))
                    .add_option(category_condition)
                    .add_option(deleted_condition),
            )
            .order_by(order.column.into_column(), order.direction)
            .all(txn)
            .await?;

        Ok(pages)
    }

    /// Checks to see if a page already exists at the slug specified.
    ///
    /// If so, this method fails with `Error::Conflict`. Otherwise it returns nothing.
    async fn check_conflicts(
        ctx: &ServiceContext<'_>,
        site_id: i64,
        slug: &str,
        action: &str,
    ) -> Result<()> {
        let txn = ctx.transaction();

        let result = Page::find()
            .filter(
                Condition::all()
                    .add(page::Column::SiteId.eq(site_id))
                    .add(page::Column::Slug.eq(slug))
                    .add(page::Column::DeletedAt.is_null()),
            )
            .one(txn)
            .await?;

        match result {
            None => Ok(()),
            Some(page) => {
                tide::log::error!(
                    "Page {} with slug '{}' already exists on site ID {}, cannot {}",
                    page.page_id,
                    slug,
                    site_id,
                    action,
                );

                Err(Error::Conflict)
            }
        }
    }

    async fn run_filter<S: AsRef<str>>(
        ctx: &ServiceContext<'_>,
        site_id: i64,
        wikitext: Option<S>,
        title: Option<S>,
        alt_title: Option<S>,
    ) -> Result<()> {
        tide::log::info!("Checking page data against filters...");

        let filter_matcher = FilterService::get_matcher(
            ctx,
            FilterClass::PlatformAndSite(site_id),
            FilterType::Page,
        )
        .await?;

        macro_rules! verify_optional {
            ($option:expr) => {
                async {
                    match $option {
                        Some(value) => filter_matcher.verify(ctx, value.as_ref()).await,
                        None => Ok(()),
                    }
                }
            };
        }

        try_join!(
            verify_optional!(title),
            verify_optional!(alt_title),
            verify_optional!(wikitext),
        )?;

        Ok(())
    }
}
