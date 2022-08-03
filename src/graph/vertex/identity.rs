use crate::{
    error::Error,
    graph::{
        edge::{Hold, HoldRecord, Proof, ProofRecord},
        vertex::Vertex,
    },
    upstream::{DataSource, Platform},
    util::naive_now,
};
use aragog::{
    query::{Comparison, Filter, Query, QueryResult},
    DatabaseConnection, DatabaseRecord, EdgeRecord, Record,
};
use arangors_lite::{AqlQuery, Database};
use serde_json::{value::Value, from_value};

use async_trait::async_trait;
use chrono::{Duration, NaiveDateTime};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize, Serialize, Record)]
#[collection_name = "Identities"]
pub struct Identity {
    /// UUID of this record. Generated by us to provide a better
    /// global-uniqueness for future P2P-network data exchange
    /// scenario.
    pub uuid: Option<Uuid>,
    /// Platform.
    pub platform: Platform,
    /// Identity on target platform.
    /// Username or database primary key (prefer, usually digits).
    /// e.g. `Twitter` has this digits-like user ID thing.
    pub identity: String,
    /// Usually user-friendly screen name.
    /// e.g. for `Twitter`, this is the user's `screen_name`.
    /// For `ethereum`, this is the reversed ENS name set by user.
    pub display_name: Option<String>,
    /// URL to target identity profile page on `platform` (if any).
    pub profile_url: Option<String>,
    /// URL to avatar (if any is recorded and given by target platform).
    pub avatar_url: Option<String>,
    /// Account / identity creation time ON TARGET PLATFORM.
    /// This is not necessarily the same as the creation time of the record in the database.
    /// Since `created_at` may not be recorded or given by target platform.
    /// e.g. `Twitter` has a `created_at` in the user profile API.
    /// but `Ethereum` is obviously no such thing.
    pub created_at: Option<NaiveDateTime>,
    /// When this Identity is added into this database. Generated by us.
    pub added_at: NaiveDateTime,
    /// When it is updated (re-fetched) by us RelationService. Managed by us.
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Path {
    pub vertices: Vec<IdentityRecord>,
    pub edges: Vec<ProofRecord>,
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            uuid: None,
            platform: Platform::Twitter,
            identity: Default::default(),
            display_name: Default::default(),
            profile_url: None,
            avatar_url: None,
            created_at: None,
            added_at: naive_now(),
            updated_at: naive_now(),
        }
    }
}

impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        self.uuid.is_some() && other.uuid.is_some() && self.uuid == other.uuid
    }
}

impl Identity {
    /// Find record by given platform and identity.
    pub async fn find_by_platform_identity(
        db: &DatabaseConnection,
        platform: &Platform,
        identity: &str,
    ) -> Result<Option<IdentityRecord>, Error> {
        let query = Self::query().filter(
            Filter::new(Comparison::field("platform").equals_str(platform))
                .and(Comparison::field("identity").equals_str(identity)),
        );
        let query_result = Self::get(&query, db).await?;

        if query_result.len() == 0 {
            Ok(None)
        } else {
            Ok(Some(query_result.first().unwrap().to_owned().into()))
        }
    }

    pub async fn find_by_platforms_identity(
        raw_db: &Database,
        platforms: &Vec<Platform>,
        identity: &str,
    ) -> Result<Vec<IdentityRecord>, Error> {
        let platform_array: Vec<Value> = platforms
            .into_iter()
            .map(|field| json!(format!("{}", field.to_string()) ))
            .rev()
            .collect();

        let aql = r"FOR v IN @@collection_name
        FILTER v.identity == @identity AND v.platform IN @platform
        RETURN v";
        let aql = AqlQuery::new(aql)
            .bind_var("@collection_name", Identity::COLLECTION_NAME)
            .bind_var("identity", identity)
            .bind_var("platform", platform_array)
            .batch_size(1)
            .count(false);
        let result: Vec<IdentityRecord> = raw_db.aql_query(aql).await?;
        Ok(result)
    }

    async fn find_by_display_name(
        raw_db: &Database,
        display_name: String,
    ) -> Result<Option<IdentityRecord>, Error> {
        let aql = r"FOR v IN relation
        FILTER v.display_name == @display_name
        RETURN v";
        let aql = AqlQuery::new(aql)
            .bind_var("display_name", display_name.as_str())
            .batch_size(1)
            .count(false);

        let result: Vec<IdentityRecord> = raw_db.aql_query(aql).await.unwrap();
        if result.len() == 0 {
            Ok(None)
        } else {
            Ok(Some(result.first().unwrap().to_owned().into()))
        }
    }
}

#[async_trait]
impl Vertex<IdentityRecord> for Identity {
    fn uuid(&self) -> Option<Uuid> {
        self.uuid
    }

    /// Do create / update side-effect.
    /// Used by upstream crawler.
    async fn create_or_update(&self, db: &DatabaseConnection) -> Result<IdentityRecord, Error> {
        // Find first
        let found = Self::find_by_platform_identity(db, &self.platform, &self.identity).await?;
        match found {
            None => {
                // Create
                let mut to_be_created = self.clone();
                to_be_created.uuid = to_be_created.uuid.or(Some(Uuid::new_v4()));
                to_be_created.added_at = naive_now();
                to_be_created.updated_at = naive_now();
                #[allow(unused_assignments)] // FIXME: ??
                let mut need_refetch: bool = false;

                // Must do this to avoid "future cannot be sent between threads safely" complain from compiler.
                match DatabaseRecord::create(to_be_created, db).await {
                    Ok(created) => return Ok(created.into()),
                    // An exception is raised from ArangoDB complaining about unique index violation.
                    // Refetch it later (after leaving this block).
                    // Since `bool` is `Send`able.
                    Err(aragog::Error::Conflict(_)) => {
                        need_refetch = true;
                    }
                    Err(err) => {
                        return Err(err.into());
                    }
                };

                if need_refetch {
                    let found =
                        Self::find_by_platform_identity(db, &self.platform, &self.identity).await?;
                    Ok(found.expect("Not found after an race condition in create_or_update"))
                } else {
                    Err(Error::General("Impossible: no refetch triggered nor record found / created in create_or_update".into(), StatusCode::INTERNAL_SERVER_ERROR))
                }
            }

            Some(mut found) => {
                // Update
                found.display_name = self.display_name.clone().or(found.display_name.clone());
                found.profile_url = self.profile_url.clone();
                found.avatar_url = self.avatar_url.clone();
                found.created_at = self.created_at.or(found.created_at);
                found.updated_at = naive_now();

                found.save(db).await?;
                Ok(found)
            }
        }
    }

    async fn find_by_uuid(
        db: &DatabaseConnection,
        uuid: Uuid,
    ) -> Result<Option<IdentityRecord>, Error> {
        let query = Identity::query().filter(Comparison::field("uuid").equals_str(uuid).into());
        let query_result = Identity::get(&query, db).await?;
        if query_result.len() == 0 {
            Ok(None)
        } else {
            Ok(Some(query_result.first().unwrap().to_owned().into()))
        }
    }

    /// Judge if this record is outdated and should be refetched.
    fn is_outdated(&self) -> bool {
        let outdated_in = Duration::hours(1);
        self.updated_at
            .checked_add_signed(outdated_in)
            .unwrap()
            .lt(&naive_now())
    }
}

/// Result struct queried from graph database.
/// Useful by GraphQL side to wrap more function / traits.
#[derive(Clone, Deserialize, Serialize, Default, Debug)]
pub struct IdentityRecord(pub DatabaseRecord<Identity>);

impl std::ops::Deref for IdentityRecord {
    type Target = DatabaseRecord<Identity>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for IdentityRecord {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<DatabaseRecord<Identity>> for IdentityRecord {
    fn from(record: DatabaseRecord<Identity>) -> Self {
        Self(record)
    }
}

impl IdentityRecord {
    /// Returns all neighbors of this identity. Depth and upstream data souce can be specified.
    pub async fn neighbors(
        &self,
        db: &DatabaseConnection,
        depth: u16,
        _source: Option<DataSource>,
    ) -> Result<Vec<Self>, Error> {
        // TODO: make `source` filter work.
        // let proof_query = match source {
        //     None => Proof::query(),
        //     Some(source) => Proof::query().filter(
        //         Comparison::field("source") // Don't know why this won't work
        //             .equals_str(source.to_string())
        //             .into(),
        //     ).distinct(),
        // };

        let result: QueryResult<Identity> = Query::any(1, depth, Proof::COLLECTION_NAME, self.id())
            .call(db)
            .await?;
        Ok(result.iter().map(|r| r.to_owned().into()).collect())
    }

    // Return all neighbors of this identity with path<ProofRecord>
    pub async fn find_neighbors_with_path(
        &self,
        raw_db: &Database,
        depth: u16,
        _source: Option<DataSource>,
    ) -> Result<Vec<Path>, Error> {
        // let aql = r"FOR d IN relation FILTER d._id == @identities_id
        // FOR vertex, edge, path IN 1..@depth OUTBOUND d Proofs
        // RETURN path";

        // Using graph can speed up from 70ms to 10ms
        match _source {
            None => {
                let aql = r"
                WITH relation
                FOR d IN Identities
                  FILTER d._id == @identities_id
                  LIMIT 1
                  FOR vertex, edge, path 
                    IN 1..@depth
                    ANY d GRAPH 'identities_proofs_graph'
                    RETURN path";

                let aql = AqlQuery::new(aql)
                    .bind_var("identities_id", self.id().as_str())
                    .bind_var("depth", depth)
                    .batch_size(1)
                    .count(false);
                let resp: Vec<Value> = raw_db.aql_query(aql).await?;
                let mut paths: Vec<Path> = Vec::new();
                for p in resp {
                    let p: Path = from_value(p).unwrap();
                    paths.push(p)
                }
                Ok(paths)
            },
            Some(source) => {
                let aql = r"
                WITH relation
                FOR d IN Identities
                  FILTER d._id == @identities_id
                  LIMIT 1
                  FOR vertex, edge, path 
                    IN 1..@depth
                    ANY d
                    GRAPH 'identities_proofs_graph'
                    FILTER path.edges[*].`source` ALL == @source
                    RETURN path";
                
                let aql = AqlQuery::new(aql)
                    .bind_var("identities_id", self.id().as_str())
                    .bind_var("depth", depth)
                    .bind_var("source", source.to_string().as_str())
                    .batch_size(1)
                    .count(false);
                let resp: Vec<Value> = raw_db.aql_query(aql).await?;
                let mut paths: Vec<Path> = Vec::new();
                for p in resp {
                    let p: Path = from_value(p).unwrap();
                    paths.push(p)
                }
                Ok(paths)
            }
        }
    }

    /// Returns all Contracts owned by this identity. Empty list if `self.platform != Ethereum`.
    pub async fn nfts(&self, db: &DatabaseConnection) -> Result<Vec<HoldRecord>, Error> {
        if self.0.record.platform != Platform::Ethereum {
            return Ok(vec![]);
        }

        let query = EdgeRecord::<Hold>::query().filter(Filter::new(
            Comparison::field("_from").equals_str(self.id()),
        ));
        let result: QueryResult<EdgeRecord<Hold>> = query.call(db).await?;
        Ok(result.iter().map(|r| r.to_owned().into()).collect())
    }

    
}

#[cfg(test)]
mod tests {

    use aragog::DatabaseConnection;
    use fake::{Dummy, Fake, Faker};
    use tokio::join;
    use uuid::Uuid;

    use super::{Identity, IdentityRecord};
    use crate::{
        error::Error,
        graph::{edge::Proof, new_db_connection, Edge, Vertex, new_raw_db_connection},
        upstream::Platform,
        util::naive_now,
    };

    impl Identity {
        /// Create test dummy data in database.
        pub async fn create_dummy(db: &DatabaseConnection) -> Result<IdentityRecord, Error> {
            let identity: Identity = Faker.fake();
            identity.create_or_update(db).await
        }
    }

    impl Dummy<Faker> for Identity {
        fn dummy_with_rng<R: rand::Rng + ?Sized>(config: &Faker, _rng: &mut R) -> Self {
            Self {
                uuid: Some(Uuid::new_v4()),
                platform: Platform::Twitter,
                identity: config.fake(),
                display_name: config.fake(),
                profile_url: Some(config.fake()),
                avatar_url: Some(config.fake()),
                created_at: Some(config.fake()),
                added_at: naive_now(),
                updated_at: naive_now(),
            }
        }
    }

    #[tokio::test]
    async fn test_create() -> Result<(), Error> {
        let identity: Identity = Faker.fake();
        let db = new_db_connection().await?;
        let result = identity.create_or_update(&db).await?;
        assert!(result.uuid.is_some());
        assert!(!result.key().is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn test_duplicated_create() -> Result<(), Error> {
        let identity: Identity = Faker.fake();
        let db = new_db_connection().await?;
        let result = identity.create_or_update(&db).await?;
        let result2 = identity
            .create_or_update(&db)
            .await
            .expect("Should created / found successfully");
        assert_eq!(result.key(), result2.key());

        Ok(())
    }

    #[tokio::test]
    async fn test_race_condition_create() -> Result<(), Error> {
        let identity: Identity = Faker.fake();
        let identity2 = identity.clone();
        let db = new_db_connection().await?;
        let db2 = db.clone();
        let session1 = async move {
            identity
                .create_or_update(&db)
                .await
                .expect("Should not return error.")
        };
        let session2 = async move {
            identity2
                .create_or_update(&db2)
                .await
                .expect("Should not return error (2nd).")
        };

        let (created1, created2) = join!(session1, session2);
        assert_eq!(created1.key(), created2.key());

        Ok(())
    }

    #[tokio::test]
    async fn test_update() -> Result<(), Error> {
        let db = new_db_connection().await?;

        let mut identity: Identity = Faker.fake();
        let created = identity.create_or_update(&db).await?;

        // Change some of data
        identity.avatar_url = Some(Faker.fake());
        identity.profile_url = Some(Faker.fake());
        let updated = identity.create_or_update(&db).await?;

        assert_eq!(created.uuid, updated.uuid);
        assert_eq!(created.key(), updated.key());
        assert_ne!(created.avatar_url, updated.avatar_url);
        assert_ne!(created.profile_url, updated.profile_url);

        Ok(())
    }

    #[tokio::test]
    async fn test_find_by_uuid() -> Result<(), Error> {
        let db = new_db_connection().await?;
        let created = Identity::create_dummy(&db).await?;
        let uuid = created.uuid.unwrap();

        let found = Identity::find_by_uuid(&db, uuid).await?;
        assert_eq!(found.unwrap().uuid, created.uuid);

        Ok(())
    }

    #[tokio::test]
    async fn test_find_by_platform_identity() -> Result<(), Error> {
        let db = new_db_connection().await?;
        let created = Identity::create_dummy(&db).await?;

        let found = Identity::find_by_platform_identity(&db, &created.platform, &created.identity)
            .await?
            .expect("Record not found");
        assert_eq!(found.uuid, created.uuid);

        Ok(())
    }

    #[tokio::test]
    async fn test_find_by_display_name() -> Result<(), Error> {
        let raw_db = new_raw_db_connection().await?;
        let found = Identity::find_by_display_name(&raw_db, String::from("0x00000003cd3aa7e760877f03275621d2692f5841"))
            .await?
            .expect("Record not found");
        println!("{:?}", found);
        Ok(())
    }

    #[tokio::test]
    async fn test_neighbors() -> Result<(), Error> {
        let db = new_db_connection().await?;
        // ID2 <--Proof1-- ID1 --Proof2--> ID3
        let id1 = Identity::create_dummy(&db).await?;
        let id2 = Identity::create_dummy(&db).await?;
        let id3 = Identity::create_dummy(&db).await?;
        let id4 = Identity::create_dummy(&db).await?;

        let proof1_raw: Proof = Faker.fake();
        let proof2_raw: Proof = Faker.fake();
        let proof3_raw: Proof = Faker.fake();
        proof1_raw.connect(&db, &id1, &id2).await?;
        proof2_raw.connect(&db, &id1, &id3).await?;
        proof3_raw.connect(&db, &id2, &id4).await?;
        let neighbors = id1.neighbors(&db, 2, None).await?;
        assert_eq!(3, neighbors.len());
        // assert!(neighbors
        //     .iter()
        //     .all(|i| i.uuid == id2.uuid || i.uuid == id3.uuid));
        Ok(())
    }

    #[tokio::test]
    async fn test_find_neighbors_with_path() -> Result<(), Error> {
        let raw_db = new_raw_db_connection().await.unwrap();
        let found = Identity::find_by_display_name(&raw_db, String::from("0x00000003cd3aa7e760877f03275621d2692f5841"))
            .await?
            .expect("Record not found");
        let neighbors = found.find_neighbors_with_path(&raw_db, 3, None).await.unwrap();
        println!("{:#?}", neighbors);
        Ok(())
    }
}
