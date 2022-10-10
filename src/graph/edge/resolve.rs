use crate::{
    error::Error,
    graph::vertex::{Contract, Identity},
    graph::Edge,
    upstream::{DataFetcher, DataSource},
    util::naive_now,
};
use aragog::{
    query::{Comparison, Filter, QueryResult},
    DatabaseConnection, DatabaseRecord, EdgeRecord, Record,
};
use chrono::{Duration, NaiveDateTime};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumIter, EnumString};
use uuid::Uuid;

#[derive(
    Clone,
    Copy,
    Serialize,
    Deserialize,
    Display,
    async_graphql::Enum,
    EnumString,
    PartialEq,
    Eq,
    EnumIter,
    Default,
)]
pub enum DomainNameSystem {
    /// ENS name system on the ETH chain.
    /// https://ens.domains
    #[strum(serialize = "ENS")]
    #[serde(rename = "ENS")]
    #[graphql(name = "ENS")]
    ENS,

    #[default]
    #[strum(serialize = "unknown")]
    #[serde(rename = "unknown")]
    #[graphql(name = "unknown")]
    Unknown,
}

/// Edge to identify which `Identity(Ethereum)` a `Contract` is resolving to.
/// Basiclly this is served for `ENS` only.
/// There're 3 kinds of relation between an `Identity(Ethereum)` and `Contract(ENS)` :
/// - `Own` relation: defined in `graph/edge/own.rs`
///   In our system: create `Own` edge from `Identity(Ethereum)` to `Contract(ENS)`.
/// - `Resolve` relation: Find an Ethereum wallet using ENS name (like DNS).
///   In our system: create `Resolve` edge from `Contract(ENS)` to `Identity(Ethereum)`.
/// - `ReverseLookup` relation: Find an ENS name using an Ethereum wallet (like reverse DNS lookup).
///   In our system: set `display_name` for the `Identity(Ethereum)`.
#[derive(Clone, Serialize, Deserialize, Record)]
#[collection_name = "Resolves"]
pub struct Resolve {
    /// UUID of this record. Generated by us to provide a better
    /// global-uniqueness for future P2P-network data exchange
    /// scenario.
    pub uuid: Uuid,
    /// Data source (upstream) which provides this connection info.
    pub source: DataSource,
    /// Domain Name system
    pub system: DomainNameSystem,
    /// Name of domain (e.g., `vitalik.eth`)
    pub name: String,
    /// Who collects this data.
    /// It works as a "data cleansing" or "proxy" between `source`s and us.
    pub fetcher: DataFetcher,
    /// When this connection is fetched by us RelationService.
    pub updated_at: NaiveDateTime,
}

impl Default for Resolve {
    fn default() -> Self {
        Self {
            uuid: Default::default(),
            source: Default::default(),
            name: Default::default(),
            system: Default::default(),
            fetcher: Default::default(),
            updated_at: naive_now(),
        }
    }
}

impl Resolve {
    pub async fn find_by_name_system(
        db: &DatabaseConnection,
        name: &str,
        system: &DomainNameSystem,
    ) -> Result<Option<ResolveRecord>, Error> {
        let filter = Filter::new(Comparison::field("system").equals_str(system))
            .and(Comparison::field("name").equals_str(name));
        let query = EdgeRecord::<Self>::query().filter(filter);
        let result: QueryResult<EdgeRecord<Self>> = query.call(db).await?;

        if result.len() == 0 {
            Ok(None)
        } else {
            Ok(Some(result.first().unwrap().clone().into()))
        }
    }

    fn is_outdated(&self) -> bool {
        let outdated_in = Duration::days(1);
        self.updated_at
            .checked_add_signed(outdated_in)
            .unwrap()
            .lt(&naive_now())
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ResolveRecord(DatabaseRecord<EdgeRecord<Resolve>>);
impl std::ops::Deref for ResolveRecord {
    type Target = DatabaseRecord<EdgeRecord<Resolve>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ResolveRecord {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<DatabaseRecord<EdgeRecord<Resolve>>> for ResolveRecord {
    fn from(record: DatabaseRecord<EdgeRecord<Resolve>>) -> Self {
        ResolveRecord(record)
    }
}

#[async_trait::async_trait]
impl Edge<Contract, Identity, ResolveRecord> for Resolve {
    fn uuid(&self) -> Option<Uuid> {
        Some(self.uuid)
    }

    async fn connect(
        &self,
        db: &DatabaseConnection,
        from: &DatabaseRecord<Contract>,
        to: &DatabaseRecord<Identity>,
    ) -> Result<ResolveRecord, Error> {
        let found = Self::find_by_name_system(db, &self.name, &self.system).await?;
        match found {
            Some(mut edge) => {
                if edge.key_from() == from.key() && edge.key_to() == to.key() {
                    // Exact the same edge. Keep it.
                    Ok(edge)
                } else {
                    // Destory old edge and create new one.
                    edge.delete(db).await?;
                    Ok(DatabaseRecord::link(from, to, db, self.clone())
                        .await?
                        .into())
                }
            }
            None => Ok(DatabaseRecord::link(from, to, db, self.clone())
                .await?
                .into()),
        }
    }

    async fn find_by_uuid(
        db: &DatabaseConnection,
        uuid: &Uuid,
    ) -> Result<Option<ResolveRecord>, Error> {
        let result: QueryResult<EdgeRecord<Self>> = EdgeRecord::<Self>::query()
            .filter(Comparison::field("uuid").equals_str(uuid).into())
            .call(db)
            .await?;

        if result.len() == 0 {
            Ok(None)
        } else {
            Ok(Some(result.first().unwrap().to_owned().into()))
        }
    }
}
