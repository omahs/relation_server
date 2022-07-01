use aragog::DatabaseConnection;
use async_graphql::{Context, Object};
use crate::error::{Error, Result};
use crate::graph::vertex::{Identity, IdentityRecord, Vertex};
use crate::upstream::fetch_all;

/// Status for a record in RelationService DB
#[derive(Default, Copy, Clone, PartialEq, Eq, async_graphql::Enum)]
enum DataStatus {
    /// Fetched or not in Database.
    #[default]
    #[graphql(name = "cached")]
    Cached,

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

        if self.key().len() > 0 {
            if self.0.record.is_outdated() {
                vec![Cached, Fetching]
            } else {
                vec![Cached]
            }
        } else {
            vec![Fetching]
        }
    }

    async fn uuid(&self) -> Option<String> {
        self.uuid.map(|u| u.to_string())
    }

    async fn platform(&self) -> String {
        self.platform.to_string()
    }

    async fn identity(&self) -> String {
        self.identity.clone()
    }

    async fn display_name(&self) -> String {
        self.display_name.clone()
    }

    async fn profile_url(&self) -> Option<String> {
        self.profile_url.clone()
    }

    async fn avatar_url(&self) -> Option<String> {
        self.avatar_url.clone()
    }

    async fn created_at(&self) -> Option<i64> {
        self.created_at.map(|dt| dt.timestamp())
    }

    async fn added_at(&self) -> i64 {
        self.added_at.timestamp()
    }

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
        #[graphql(
            desc = "Depth of traversal. 1 if omitted",
        )]
        depth: Option<u16>,
    ) -> Result<Vec<IdentityRecord>> {
        let db: &DatabaseConnection = ctx.data().map_err(|err| Error::GraphQLError(err.message))?;
        self.neighbors(
            db,
            depth.unwrap_or(1),
            // upstream.map(|u| DataSource::from_str(&u).unwrap_or(DataSource::Unknown))
            None
        ).await
    }
}

#[derive(Default)]
pub struct IdentityQuery;

#[Object]
impl IdentityQuery {
    /// Query an `identity` by given `platform` and `identity`.
    async fn identity(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Platform to query")] platform: String,
        #[graphql(desc = "Identity on target Platform")] identity: String,
    ) -> Result<Option<IdentityRecord>> {
        let db: &DatabaseConnection = ctx.data().map_err(|err| Error::GraphQLError(err.message))?;
        let platform = platform.parse()?;
        // FIXME: Super dirty. Should be in an async job/worker-like shape.
        match Identity::find_by_platform_identity(&db, &platform, &identity).await? {
            None => {
                fetch_all(&platform, &identity).await?;
                Identity::find_by_platform_identity(&db, &platform, &identity).await
            },
            Some(found) => {
                if found.0.record.is_outdated() {
                    fetch_all(&platform, &identity).await?;
                    Identity::find_by_platform_identity(&db, &platform, &identity).await
                } else {
                    Ok(Some(found))
                }
            },
        }
    }
}
