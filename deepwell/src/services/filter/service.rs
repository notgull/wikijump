/*
 * services/filter/service.rs
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
use crate::models::filter::{self, Entity as Filter, Model as FilterModel};
use regex::{Regex, RegexSet};

#[derive(Debug)]
pub struct FilterService;

impl FilterService {
    pub async fn create(
        ctx: &ServiceContext<'_>,
        site_id: Option<i64>,
        CreateFilter {
            affects_user,
            affects_email,
            affects_page,
            affects_file,
            affects_forum,
            regex,
            description,
        }: CreateFilter,
    ) -> Result<FilterModel> {
        let txn = ctx.transaction();

        tide::log::info!("Creating filter with regex '{regex}' because '{description}'");

        // Ensure the regular expression is valid
        if Regex::new(&regex).is_err() {
            tide::log::error!("Passed regular expression pattern is invalid: {regex}");
            return Err(Error::BadRequest);
        }

        // Ensure there aren't conflicts
        Self::check_conflicts(ctx, site_id, &regex, "create").await?;

        let model = filter::ActiveModel {
            site_id: Set(site_id),
            affects_user: Set(affects_user),
            affects_email: Set(affects_email),
            affects_page: Set(affects_page),
            affects_file: Set(affects_file),
            affects_forum: Set(affects_forum),
            regex: Set(regex),
            description: Set(description),
            ..Default::default()
        };
        let filter = model.insert(txn).await?;
        Ok(filter)
    }

    #[allow(dead_code)] // TEMP
    pub async fn update(
        ctx: &ServiceContext<'_>,
        UpdateFilter {
            filter_id,
            affects_user,
            affects_email,
            affects_page,
            affects_file,
            affects_forum,
            regex,
            description,
        }: UpdateFilter,
    ) -> Result<FilterModel> {
        let txn = ctx.transaction();

        tide::log::info!("Updating filter with ID {filter_id}");

        let mut model = filter::ActiveModel {
            filter_id: Set(filter_id),
            updated_at: Set(Some(now())),
            ..Default::default()
        };

        // Set fields
        if let ProvidedValue::Set(affects) = affects_user {
            model.affects_user = Set(affects);
        }

        if let ProvidedValue::Set(affects) = affects_email {
            model.affects_email = Set(affects);
        }

        if let ProvidedValue::Set(affects) = affects_page {
            model.affects_page = Set(affects);
        }

        if let ProvidedValue::Set(affects) = affects_file {
            model.affects_file = Set(affects);
        }

        if let ProvidedValue::Set(affects) = affects_forum {
            model.affects_forum = Set(affects);
        }

        if let ProvidedValue::Set(regex) = regex {
            model.regex = Set(regex);
        }

        if let ProvidedValue::Set(description) = description {
            model.description = Set(description);
        }

        // Perform update
        let filter = model.update(txn).await?;
        Ok(filter)
    }

    #[allow(dead_code)] // TEMP
    pub async fn delete(ctx: &ServiceContext<'_>, filter_id: i64) -> Result<()> {
        let txn = ctx.transaction();

        tide::log::info!("Deleting filter with ID {filter_id}");

        // Ensure filter exists
        let filter = Self::get(ctx, filter_id).await?;
        if filter.deleted_at.is_some() {
            tide::log::error!("Attempting to delete already-deleted filter");
            return Err(Error::BadRequest);
        }

        // Delete the filter
        let model = filter::ActiveModel {
            filter_id: Set(filter_id),
            deleted_at: Set(Some(now())),
            ..Default::default()
        };
        model.update(txn).await?;
        Ok(())
    }

    /// Restores a filter, causing it to be undeleted.
    #[allow(dead_code)] // TEMP
    pub async fn restore(
        ctx: &ServiceContext<'_>,
        filter_id: i64,
    ) -> Result<FilterModel> {
        let txn = ctx.transaction();

        tide::log::info!("Undeleting filter with ID {filter_id}");

        let filter = Self::get(ctx, filter_id).await?;
        if filter.deleted_at.is_none() {
            tide::log::error!("Attempting to un-delete extant filter");
            return Err(Error::BadRequest);
        }

        // Ensure it doesn't conflict with a since-added filter
        Self::check_conflicts(ctx, filter.site_id, &filter.regex, "restore").await?;

        // Un-delete the filter
        let model = filter::ActiveModel {
            filter_id: Set(filter_id),
            deleted_at: Set(None),
            ..Default::default()
        };
        let filter = model.update(txn).await?;
        Ok(filter)
    }

    #[inline]
    pub async fn get(ctx: &ServiceContext<'_>, filter_id: i64) -> Result<FilterModel> {
        find_or_error(Self::get_optional(ctx, filter_id)).await
    }

    pub async fn get_optional(
        ctx: &ServiceContext<'_>,
        filter_id: i64,
    ) -> Result<Option<FilterModel>> {
        tide::log::info!("Getting filter with ID {filter_id}");

        let txn = ctx.transaction();
        let filter = Filter::find_by_id(filter_id).one(txn).await?;
        Ok(filter)
    }

    /// Get all filters of a type.
    ///
    /// For the `filter_class` argument, see `FilterClass`.
    ///
    /// The `filter_type` argument:
    /// * If it is `Some(_)`, it determines what kind of object is being filtered.
    /// * If it is `None`, then it returns everything.
    ///
    /// The `deleted` argument:
    /// * If it is `Some(true)`, then it only returns filters which have been deleted.
    /// * If it is `Some(false)`, then it only returns filters which are extant.
    /// * If it is `None`, then it returns all filters regardless of deletion status.
    pub async fn get_all(
        ctx: &ServiceContext<'_>,
        filter_class: FilterClass,
        filter_type: Option<FilterType>,
        deleted: Option<bool>,
    ) -> Result<Vec<FilterModel>> {
        let txn = ctx.transaction();

        tide::log::info!("Getting all {} filters", filter_class.name());

        let filter_condition =
            filter_type.map(|filter_type| filter_type.into_column().eq(true));

        let deleted_condition = match deleted {
            Some(true) => Some(filter::Column::DeletedAt.is_not_null()),
            Some(false) => Some(filter::Column::DeletedAt.is_null()),
            None => None,
        };

        let filters = Filter::find()
            .filter(
                Condition::all()
                    .add(filter_class.to_condition())
                    .add_option(filter_condition)
                    .add_option(deleted_condition),
            )
            .all(txn)
            .await?;

        Ok(filters)
    }

    /// Get all filters of a type, specifically extracting the regular expressions.
    ///
    /// This only pulls extant filters, as those are the only ones which are enforced.
    // TODO cache this somehow
    //      maybe so that it stores the RegexSet and deletes it if an insert/update/etc
    //      above occurs to that filter class/type
    pub async fn get_matcher(
        ctx: &ServiceContext<'_>,
        filter_class: FilterClass,
        filter_type: FilterType,
    ) -> Result<FilterMatcher> {
        tide::log::info!(
            "Compiling regex set for {} filters for {filter_type:?}",
            filter_class.name(),
        );

        let filters =
            Self::get_all(ctx, filter_class, Some(filter_type), Some(false)).await?;

        let mut regexes = Vec::new();
        let mut filter_data = Vec::new();

        for FilterModel {
            filter_id,
            regex,
            description,
            ..
        } in filters
        {
            regexes.push(regex);
            filter_data.push(FilterSummary {
                filter_id,
                description,
            });
        }

        let regex_set = RegexSet::new(regexes).map_err(|error| {
            tide::log::error!(
                "Invalid regular expression found in the database: {error}",
            );

            Error::Inconsistent
        })?;

        Ok(FilterMatcher::new(regex_set, filter_data))
    }

    /// Checks if creating / reinstating this filter would cause constraint violations.
    async fn check_conflicts(
        ctx: &ServiceContext<'_>,
        site_id: Option<i64>,
        regex: &str,
        action: &str,
    ) -> Result<()> {
        let txn = ctx.transaction();

        let result = Filter::find()
            .filter(
                Condition::all()
                    .add(filter::Column::SiteId.eq(site_id))
                    .add(filter::Column::Regex.eq(regex))
                    .add(filter::Column::DeletedAt.is_null()),
            )
            .one(txn)
            .await?;

        match result {
            None => Ok(()),
            Some(_) => {
                tide::log::error!(
                    " filter '{regex}' for {site_id:?} already exists, cannot {action}"
                );
                Err(Error::Conflict)
            }
        }
    }
}
