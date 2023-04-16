/*
 * services/domain/service.rs
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

//! Service for managing domains as used by Wikijump sites.
//!
//! This service has two components, management of canonical domains (e.g. `scp-wiki.wikijump.com`)
//! and custom domains (e.g. `scpwiki.com`).

// TODO disallow custom domains that are subdomains of the main domain or files domain

use super::prelude::*;
use crate::models::site::{self, Entity as Site, Model as SiteModel};
use crate::models::site_domain::{self, Entity as SiteDomain, Model as SiteDomainModel};
use crate::services::SiteService;

#[derive(Debug)]
pub struct DomainService;

impl DomainService {
    /// Creates a custom domain for a site.
    pub async fn create_custom(
        ctx: &ServiceContext<'_>,
        CreateCustomDomain { domain, site_id }: CreateCustomDomain,
    ) -> Result<()> {
        tide::log::info!("Creating custom domain '{domain}' (site ID {site_id})");

        let txn = ctx.transaction();
        if Self::custom_domain_exists(ctx, &domain).await? {
            tide::log::error!("Custom domain already exists, cannot create");
            return Err(Error::Conflict);
        }

        let model = site_domain::ActiveModel {
            domain: Set(domain),
            site_id: Set(site_id),
            created_at: Set(now()),
        };
        model.insert(txn).await?;
        Ok(())
    }

    /// Delete the given custom domain.
    ///
    /// Yields `Error::NotFound` if it's missing.
    pub async fn delete_custom(ctx: &ServiceContext<'_>, domain: String) -> Result<()> {
        tide::log::info!("Deleting custom domain '{domain}'");

        let txn = ctx.transaction();
        let DeleteResult { rows_affected, .. } =
            SiteDomain::delete_by_id(domain).exec(txn).await?;

        if rows_affected == 1 {
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }

    pub async fn site_from_custom_domain_optional(
        ctx: &ServiceContext<'_>,
        domain: &str,
    ) -> Result<Option<SiteModel>> {
        tide::log::info!("Getting site for custom domain '{domain}'");

        // Join with the site table so we can get that data, rather than just the ID.
        let txn = ctx.transaction();
        let model = Site::find()
            .join(JoinType::Join, site::Relation::SiteDomain.def())
            .filter(site_domain::Column::Domain.eq(domain))
            .one(txn)
            .await?;

        Ok(model)
    }

    #[inline]
    pub async fn site_from_custom_domain(
        ctx: &ServiceContext<'_>,
        domain: &str,
    ) -> Result<SiteModel> {
        find_or_error(Self::site_from_custom_domain_optional(ctx, domain)).await
    }

    /// Determines if the given custom domain is registered.
    #[inline]
    pub async fn custom_domain_exists(
        ctx: &ServiceContext<'_>,
        domain: &str,
    ) -> Result<bool> {
        Self::site_from_custom_domain_optional(ctx, domain)
            .await
            .map(|site| site.is_some())
    }

    /// If this domain is canonical domain, extract the site slug.
    pub fn parse_canonical<'a>(config: &Config, domain: &'a str) -> Option<&'a str> {
        let main_domain = &config.main_domain;
        match domain.strip_prefix(main_domain) {
            // Only 1-deep subdomains of the main domain are allowed.
            // For instance, foo.wikijump.com or bar.wikijump.com are valid,
            // but foo.bar.wikijump.com is not.
            Some(subdomain) if subdomain.contains('.') => {
                tide::log::error!("Found domain '{domain}' is a sub-subdomain, invalid");
                None
            }

            Some(subdomain) => Some(subdomain),
            None => None,
        }
    }

    #[inline]
    pub fn get_canonical(config: &Config, site_slug: &str) -> String {
        // 'main_domain' already is prefixed with .
        format!("{}{}", site_slug, config.main_domain)
    }

    /// Optional version of `site_from_domain()`.
    pub async fn site_from_domain_optional<'a>(
        ctx: &ServiceContext<'_>,
        domain: &'a str,
    ) -> Result<(Option<SiteModel>, Option<&'a str>)> {
        tide::log::info!("Getting site for domain '{domain}'");

        match Self::parse_canonical(ctx.config(), domain) {
            // Normal canonical domain, return from site slug fetch.
            Some(subdomain) => {
                tide::log::debug!("Found canonical domain with slug '{subdomain}'");
                let site =
                    SiteService::get_optional(ctx, Reference::Slug(cow!(subdomain)))
                        .await?;

                Ok((site, Some(subdomain)))
            }

            // Not canonical, try custom domain.
            None => {
                tide::log::debug!("Not found, checking if it's a custom domain");
                let site = Self::site_from_custom_domain_optional(ctx, domain).await?;
                Ok((site, None))
            }
        }
    }

    /// Gets the site corresponding with the given domain.
    ///
    /// # Returns
    /// A 2-tuple, the first containing the site for this domain,
    /// the second containing the site slug in this domain
    /// (or `None` if it was a custom domain).
    #[inline]
    pub async fn site_from_domain<'a>(
        ctx: &ServiceContext<'_>,
        domain: &'a str,
    ) -> Result<(SiteModel, Option<&'a str>)> {
        match Self::site_from_domain_optional(ctx, domain).await? {
            (Some(site), site_slug) => Ok((site, site_slug)),
            (None, _) => Err(Error::NotFound),
        }
    }

    /// Gets the preferred domain for the given site.
    pub async fn domain_for_site(ctx: &ServiceContext<'_>, site: &SiteModel) -> String {
        tide::log::debug!(
            "Getting preferred domain for site '{}' (ID {})",
            site.slug,
            site.site_id,
        );

        match &site.custom_domain {
            Some(domain) => str!(domain),
            None => format!("{}{}", site.slug, ctx.config().main_domain),
        }
    }

    /// Gets all custom domains for a site.
    pub async fn list_custom(
        ctx: &ServiceContext<'_>,
        site_id: i64,
    ) -> Result<Vec<SiteDomainModel>> {
        tide::log::info!("Getting domains for site ID {site_id}");

        let txn = ctx.transaction();
        let models = SiteDomain::find()
            .filter(site_domain::Column::SiteId.eq(site_id))
            .all(txn)
            .await?;

        Ok(models)
    }
}
