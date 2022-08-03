use crate::error::{Error, Result};
use crate::graph::edge::HoldRecord;
use crate::graph::vertex::{Identity, IdentityRecord, Vertex};
use crate::upstream::{fetch_all, DataSource, Platform, Target};

use aragog::DatabaseConnection;
use async_graphql::{Context, Object};
use arangors_lite::Database;
use log::debug;
use strum::IntoEnumIterator;

/// Status for a record in RelationService DB
#[derive(Default, Copy, Clone, PartialEq, Eq, async_graphql::Enum)]
enum DataStatus {
    /// Fetched or not in Database.
    #[default]
    #[graphql(name = "cached")]
    Cached,

    /// Outdated record
    #[graphql(name = "outdated")]
    Outdated,

    /// Fetching this data.
    /// The result you got maybe outdated.
    /// Come back later if you want a fresh one.
    #[graphql(name = "fetching")]
    Fetching,
}

#[Object]
impl IdentityRecord {
    /// Status for this record in RelationService.
    async fn status(&self) -> Vec<DataStatus> {
        use DataStatus::*;
        let mut current: Vec<DataStatus> = vec![];
        if !self.key().is_empty() {
            current.push(Cached);
            if self.is_outdated() {
                current.push(Outdated);
            }
        } else {
            current.push(Fetching); // FIXME: Seems like this is never reached.
        }
        current
    }

    /// UUID of this record.  Generated by us to provide a better
    /// global-uniqueness for future P2P-network data exchange
    /// scenario.
    async fn uuid(&self) -> Option<String> {
        self.uuid.map(|u| u.to_string())
    }

    /// Platform.  See `avaliablePlatforms` or schema definition for a
    /// list of platforms supported by RelationService.
    async fn platform(&self) -> Platform {
        self.platform
    }

    /// Identity on target platform.  Username or database primary key
    /// (prefer, usually digits).  e.g. `Twitter` has this digits-like
    /// user ID thing.
    async fn identity(&self) -> String {
        self.identity.clone()
    }

    /// Usually user-friendly screen name.  e.g. for `Twitter`, this
    /// is the user's `screen_name`.
    /// Note: both `null` and `""` should be treated as "no value".
    async fn display_name(&self) -> Option<String> {
        self.display_name.clone()
    }

    /// URL to target identity profile page on `platform` (if any).
    async fn profile_url(&self) -> Option<String> {
        self.profile_url.clone()
    }

    /// URL to avatar (if any is recorded and given by target platform).
    async fn avatar_url(&self) -> Option<String> {
        self.avatar_url.clone()
    }

    /// Account / identity creation time ON TARGET PLATFORM.
    /// This is not necessarily the same as the creation time of the record in the database.
    /// Since `created_at` may not be recorded or given by target platform.
    /// e.g. `Twitter` has a `created_at` in the user profile API.
    /// but `Ethereum` is obviously no such thing.
    async fn created_at(&self) -> Option<i64> {
        self.created_at.map(|dt| dt.timestamp())
    }

    /// When this Identity is added into this database.
    /// Second-based unix timestamp.
    /// Generated by us.
    async fn added_at(&self) -> i64 {
        self.added_at.timestamp()
    }

    /// When it is updated (re-fetched) by us RelationService.
    /// Second-based unix timestamp.
    /// Managed by us.
    async fn updated_at(&self) -> i64 {
        self.updated_at.timestamp()
    }

    /// Neighbor identity from current. Flattened.
    async fn neighbor(
        &self,
        ctx: &Context<'_>,
        // #[graphql(
        //     desc = "Upstream source of this connection. Will search all upstreams if omitted."
        // )]
        // upstream: Option<String>,
        #[graphql(desc = "Depth of traversal. 1 if omitted")] depth: Option<u16>,
    ) -> Result<Vec<IdentityRecord>> {
        let db: &DatabaseConnection = ctx.data().map_err(|err| Error::GraphQLError(err.message))?;
        self.neighbors(
            db,
            depth.unwrap_or(1),
            // upstream.map(|u| DataSource::from_str(&u).unwrap_or(DataSource::Unknown))
            None,
        )
        .await
    }

    /// NFTs owned by this identity.
    /// For now, there's only `platform: ethereum` identity has NFTs.
    async fn nft(&self, ctx: &Context<'_>) -> Result<Vec<HoldRecord>> {
        let db: &DatabaseConnection = ctx.data().map_err(|err| Error::GraphQLError(err.message))?;
        self.nfts(db).await
    }
}

#[derive(Default)]
pub struct IdentityQuery;

#[Object]
impl IdentityQuery {
    /// Returns a list of all platforms supported by RelationService.
    async fn available_platforms(&self) -> Result<Vec<String>> {
        Ok(Platform::iter().map(|p| p.to_string()).collect())
    }

    /// Returns a list of all upstreams (data sources) supported by RelationService.
    async fn available_upstreams(&self) -> Result<Vec<String>> {
        Ok(DataSource::iter().map(|p| p.to_string()).collect())
    }

    /// Query an `identity` by given `platform` and `identity`.
    async fn identity(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Platform to query")] platform: String,
        #[graphql(desc = "Identity on target Platform")] identity: String,
    ) -> Result<Option<IdentityRecord>> {
        let db: &DatabaseConnection = ctx.data().map_err(|err| Error::GraphQLError(err.message))?;
        let platform: Platform = platform.parse()?;
        let target = Target::Identity(platform, identity.clone());
        // FIXME: Still kinda dirty. Should be in an background queue/worker-like shape.
        match Identity::find_by_platform_identity(db, &platform, &identity).await? {
            None => {
                fetch_all(target).await?;
                Identity::find_by_platform_identity(db, &platform, &identity).await
            }
            Some(found) => {
                if found.is_outdated() {
                    debug!(
                        "Identity: {}/{} is outdated. Refetching...",
                        platform, identity
                    );
                    tokio::spawn(async move { fetch_all(target).await });
                }
                Ok(Some(found))
            }
        }
    }

    async fn identities(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Platform array to query")] platforms: Vec<String>,
        #[graphql(desc = "Identity on target Platform")] identity: String,
    ) -> Result<Vec<IdentityRecord>> {
        let raw_db: &Database = ctx.data().map_err(|err| Error::GraphQLError(err.message))?;

        let platform_list : Vec<String> = Platform::iter().map(|p| p.to_string()).rev().collect();
        let platforms : Vec<Platform> = platforms
            .into_iter()
            .map(|field| field.parse()
            .expect(format!("Platforms should in [{}]", platform_list.join(", ")).as_str())
        ).rev().collect();

        let record: Vec<IdentityRecord> = Identity::find_by_platforms_identity(&raw_db, &platforms, &identity).await?;
        if record.len() == 0 {
            for platform in &platforms {
                let target = Target::Identity(platform.clone(), identity.clone());
                fetch_all(target).await?;
            }
            return Identity::find_by_platforms_identity(&raw_db, &platforms, &identity).await
        } else {
            for r in &record {
                if r.is_outdated() {
                    let target = Target::Identity(r.platform.clone(), identity.clone());
                    debug!(
                        "Identity: {}/{} is outdated. Refetching...",
                        r.platform, identity
                    );
                    tokio::spawn(async move { fetch_all(target).await });
                }
            }
            Ok(record)
        }
    }
}
