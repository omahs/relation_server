use crate::{
    error::Error,
    graph::ConnectionPool,
    graph::{
        edge::{Hold, HoldRecord, Proof, ProofRecord},
        vertex::vec_string_to_vec_datasource,
        vertex::Vertex,
    },
    upstream::{DataSource, Platform},
    util::naive_now,
};
use aragog::{
    query::{Comparison, Filter},
    DatabaseConnection, DatabaseRecord, Record,
};
use arangors_lite::AqlQuery;
use array_tool::vec::Uniq;
use async_trait::async_trait;
use chrono::{Duration, NaiveDateTime};
use dataloader::BatchFn;
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{from_value, json, value::Value};
use std::collections::HashMap;
use tracing::debug;
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

        /* Use connection pool
        let db = pool.db().await?;
        let aql = r"FOR v IN @@collection_name
        FILTER v.identity == @identity AND v.platform == @platform
        RETURN v";
        let aql = AqlQuery::new(aql)
            .bind_var("@collection_name", Identity::COLLECTION_NAME)
            .bind_var("identity", identity)
            .bind_var("platform", platform.to_string())
            .batch_size(1)
            .count(false);

        let result: Vec<IdentityRecord> = db.aql_query(aql).await?;
        if result.len() == 0 {
            Ok(None)
        } else {
            Ok(Some(result.first().unwrap().to_owned().into()))
        }
        */
    }

    pub async fn find_by_platforms_identity(
        pool: &ConnectionPool,
        platforms: &Vec<Platform>,
        identity: &str,
    ) -> Result<Vec<IdentityRecord>, Error> {
        let db = pool.db().await?;
        let platform_array: Vec<Value> = platforms
            .into_iter()
            .map(|field| json!(field.to_string()))
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
        let result: Vec<IdentityRecord> = db.aql_query(aql).await?;
        Ok(result)
    }

    #[allow(unused)]
    async fn find_by_display_name(
        pool: &ConnectionPool,
        display_name: String,
    ) -> Result<Option<IdentityRecord>, Error> {
        let db = pool.db().await?;
        let aql = r"FOR v IN @@collection_name
        SEARCH ANALYZER(v.display_name IN TOKENS(@display_name, 'text_en'), 'text_en')
        RETURN v";

        let aql = AqlQuery::new(aql)
            .bind_var("@collection_name", Identity::COLLECTION_NAME)
            .bind_var("display_name", display_name.as_str())
            .batch_size(1)
            .count(false);

        let result: Vec<IdentityRecord> = db.aql_query(aql).await?;
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
                    // FIXME: `.except()` below DOES have chance to be triggered. Really should take a look at the whole fn.
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

#[derive(Clone, Deserialize, Serialize, Default, Debug)]
pub struct ToIdentityRecord {
    /// NFT_ID of ENS is a hash of domain. So domain can be used as NFT_ID.
    pub id: String,
    /// Account / identity Holds NFT -> Contract
    pub identity: IdentityRecord,
}

#[derive(Clone, Deserialize, Serialize, Default, Debug)]
pub struct IdentityWithSource {
    pub identity: IdentityRecord,
    pub sources: Vec<DataSource>,
}

#[derive(Clone, Deserialize, Serialize, Default, Debug)]
pub struct FromToRecord {
    /// ProofRecord _id
    pub id: String,
    /// Two vertices of proof
    pub from_v: IdentityRecord,
    pub to_v: IdentityRecord,
}

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

pub struct FromToLoadFn {
    pub pool: ConnectionPool,
}

#[async_trait::async_trait]
impl BatchFn<String, Option<(IdentityRecord, IdentityRecord)>> for FromToLoadFn {
    async fn load(
        &mut self,
        ids: &[String],
    ) -> HashMap<String, Option<(IdentityRecord, IdentityRecord)>> {
        debug!("Loading proof for: {:?}", ids);
        let records = get_from_to_record(&self.pool, ids.to_vec()).await;
        match records {
            Ok(records) => records,
            // HOLD ON: Not sure if `Err` need to return
            Err(_) => ids.iter().map(|k| (k.to_owned(), None)).collect(),
        }
    }
}

pub struct IdentityLoadFn {
    pub pool: ConnectionPool,
}

#[async_trait::async_trait]
impl BatchFn<String, Option<IdentityRecord>> for IdentityLoadFn {
    async fn load(&mut self, ids: &[String]) -> HashMap<String, Option<IdentityRecord>> {
        debug!("Loading Identity for: {:?}", ids);
        let identities = get_identities(&self.pool, ids.to_vec()).await;
        match identities {
            Ok(identities) => identities,
            // HOLD ON: Not sure if `Err` need to return
            Err(_) => ids.iter().map(|k| (k.to_owned(), None)).collect(),
        }
    }
}

/// It already returns Dataloader friendly output given the NFT IDs.
async fn get_identities(
    pool: &ConnectionPool,
    ids: Vec<String>,
) -> Result<HashMap<String, Option<IdentityRecord>>, Error> {
    let db = pool.db().await?;
    let nft_ids: Vec<Value> = ids.iter().map(|field| json!(field.to_string())).collect();

    let aql = r###"WITH @@edge_collection_name
    FOR d IN @@edge_collection_name
        FILTER d.id IN @nft_ids
        LET v = d._from
        FOR i IN @@collection_name FILTER i._id == v
        RETURN {"id": d.id, "identity": i}"###;

    let aql = AqlQuery::new(aql)
        .bind_var("@edge_collection_name", Hold::COLLECTION_NAME)
        .bind_var("@collection_name", Identity::COLLECTION_NAME)
        .bind_var("nft_ids", nft_ids)
        .batch_size(1)
        .count(false);

    let identities = db.aql_query::<ToIdentityRecord>(aql).await;
    match identities {
        Ok(contents) => {
            let id_identities_map = contents
                .into_iter()
                .map(|content| (content.id.clone(), Some(content.identity)))
                .collect();

            let dataloader_map = ids.into_iter().fold(
                id_identities_map,
                |mut map: HashMap<String, Option<IdentityRecord>>, id| {
                    map.entry(id).or_insert(None);
                    map
                },
            );

            Ok(dataloader_map)
        }
        Err(e) => Err(Error::ArangoLiteDBError(e)),
    }
}

async fn get_from_to_record(
    pool: &ConnectionPool,
    ids: Vec<String>,
) -> Result<HashMap<String, Option<(IdentityRecord, IdentityRecord)>>, Error> {
    let db = pool.db().await?;
    let proof_ids: Vec<Value> = ids.iter().map(|field| json!(field.to_string())).collect();
    let aql_str = r###"WITH @@edge_collection_name
    FOR d IN @@edge_collection_name
        FILTER d._id IN @proof_ids
            FOR from_v IN @@collection_name FILTER from_v._id == d._from
            FOR to_v IN @@collection_name FILTER to_v._id == d._to
        RETURN {"id": d._id, "from_v": from_v, "to_v": to_v}"###;

    let aql = AqlQuery::new(aql_str)
        .bind_var("@edge_collection_name", Proof::COLLECTION_NAME)
        .bind_var("@collection_name", Identity::COLLECTION_NAME)
        .bind_var("proof_ids", proof_ids.clone())
        .batch_size(1)
        .count(false);

    let edges = db.aql_query::<FromToRecord>(aql).await;
    match edges {
        Ok(contents) => {
            let id_tuple_map = contents
                .into_iter()
                .map(|content| (content.id.clone(), Some((content.from_v, content.to_v))))
                .collect();
            let dataloader_map = ids.into_iter().fold(
                id_tuple_map,
                |mut map: HashMap<String, Option<(IdentityRecord, IdentityRecord)>>, id| {
                    map.entry(id).or_insert(None);
                    map
                },
            );
            Ok(dataloader_map)
        }
        Err(e) => Err(Error::ArangoLiteDBError(e)),
    }
}

impl IdentityRecord {
    /// Returns all neighbors of this identity. Depth and upstream data souce can be specified.
    pub async fn neighbors(
        &self,
        pool: &ConnectionPool,
        depth: u16,
        _source: Option<DataSource>,
    ) -> Result<Vec<IdentityWithSource>, Error> {
        let db = pool.db().await?;
        let aql_str = r"
        WITH @@collection_name FOR d IN @@collection_name
          FILTER d._id == @id
          LIMIT 1
          FOR vertex, edge, path
            IN 1..@depth
            ANY d GRAPH @graph_name
            RETURN path";

        let aql = AqlQuery::new(aql_str)
            .bind_var("@collection_name", Identity::COLLECTION_NAME)
            .bind_var("graph_name", "identities_proofs_graph")
            .bind_var("id", self.id().as_str())
            .bind_var("depth", depth)
            .batch_size(1)
            .count(false);

        let resp: Vec<Value> = db.aql_query(aql).await?;
        let mut identity_map: HashMap<String, IdentityRecord> = HashMap::new();
        let mut sources_map: HashMap<String, Vec<String>> = HashMap::new();
        for p in resp {
            let path: Path = from_value(p)?;

            let last = path.vertices.last().unwrap().to_owned();
            let last_edge = path.edges.last().unwrap().to_owned();
            let key = last.id().to_string();
            identity_map.entry(key.clone()).or_insert(last);
            sources_map
                .entry(key.clone())
                .or_insert_with(|| Vec::new())
                .push(last_edge.source.to_string());
        }

        let mut identity_sources: Vec<IdentityWithSource> = Vec::new();
        for (k, v) in &identity_map {
            let source_list = sources_map.get(k);
            match source_list {
                Some(ss) => {
                    let unique_sources = ss.to_owned().unique();
                    // debug!("unique_sources = {:#?}", unique_sources);
                    let _sources = vec_string_to_vec_datasource(unique_sources)?;
                    if _sources.len() > 0 {
                        let id = IdentityWithSource {
                            sources: _sources,
                            identity: v.to_owned(),
                        };
                        identity_sources.push(id);
                    }
                }
                None => continue,
            };
        }
        Ok(identity_sources)
    }

    // Return lens owned by wallet address.
    pub async fn lens_owned_by(
        &self,
        pool: &ConnectionPool,
    ) -> Result<Option<IdentityRecord>, Error> {
        let db = pool.db().await?;
        let aql_str = r"
          WITH @@collection_name
            FOR d IN @@collection_name
            FILTER d._id == @id AND d.platform == @platform
            LIMIT 1
            FOR vertex
                IN 1..1 ANY d GRAPH @graph_name
                RETURN DISTINCT vertex";

        let aql = AqlQuery::new(aql_str)
            .bind_var("@collection_name", Identity::COLLECTION_NAME)
            .bind_var("graph_name", "identities_proofs_graph")
            .bind_var("id", self.id().as_str())
            .bind_var("platform", "lens")
            .batch_size(1)
            .count(false);

        let result = db.aql_query::<IdentityRecord>(aql).await?;

        if result.len() == 0 {
            Ok(None)
        } else {
            Ok(Some(result.first().unwrap().to_owned().into()))
        }
    }

    // Return all neighbors of this identity with path<ProofRecord>
    pub async fn neighbors_with_traversal(
        &self,
        pool: &ConnectionPool,
        depth: u16,
        source: Option<DataSource>,
    ) -> Result<Vec<ProofRecord>, Error> {
        // Using graph speed up FILTER
        let db = pool.db().await?;
        let aql: AqlQuery;
        match source {
            None => {
                let aql_str = r"
                WITH @@collection_name FOR d IN @@collection_name
                  FILTER d._id == @id
                  LIMIT 1
                  FOR vertex, edge, path
                    IN 1..@depth
                    ANY d GRAPH @graph_name
                    RETURN DISTINCT edge";

                aql = AqlQuery::new(aql_str)
                    .bind_var("@collection_name", Identity::COLLECTION_NAME)
                    .bind_var("graph_name", "identities_proofs_graph")
                    .bind_var("id", self.id().as_str())
                    .bind_var("depth", depth)
                    .batch_size(1)
                    .count(false);
            }
            Some(source) => {
                let aql_str = r"
                WITH @@collection_name FOR d IN @@collection_name
                  FILTER d._id == @id
                  LIMIT 1
                  FOR vertex, edge, path
                    IN 1..@depth
                    ANY d GRAPH @graph_name
                    FILTER path.edges[*].`source` ALL == @source
                    RETURN edge";

                aql = AqlQuery::new(aql_str)
                    .bind_var("@collection_name", Identity::COLLECTION_NAME)
                    .bind_var("graph_name", "identities_proofs_graph")
                    .bind_var("id", self.id().as_str())
                    .bind_var("depth", depth)
                    .bind_var("source", source.to_string().as_str())
                    .batch_size(1)
                    .count(false);
            }
        }
        let resp: Vec<Value> = db.aql_query(aql).await?;
        let mut paths: Vec<ProofRecord> = Vec::new();
        for p in resp {
            let p: ProofRecord = from_value(p)?;
            paths.push(p)
        }
        Ok(paths)
    }

    /// Returns all Contracts owned by this identity. Empty list if `self.platform != Ethereum`.
    pub async fn nfts(&self, pool: &ConnectionPool) -> Result<Vec<HoldRecord>, Error> {
        if self.0.record.platform != Platform::Ethereum {
            return Ok(vec![]);
        }

        let db = pool.db().await?;
        let aql_str = r"WITH @@edge_collection_name
            FOR d in @@edge_collection_name
            FILTER d._from == @id
            RETURN d";
        let aql = AqlQuery::new(aql_str)
            .bind_var("@edge_collection_name", Hold::COLLECTION_NAME)
            .bind_var("id", self.id().as_str())
            .batch_size(1)
            .count(false);

        let result = db.aql_query::<HoldRecord>(aql).await?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {

    use crate::graph::vertex::identity::get_identities;
    use aragog::DatabaseConnection;
    use fake::{Dummy, Fake, Faker};
    use tokio::join;
    use uuid::Uuid;

    use super::{Identity, IdentityRecord};
    use crate::{
        error::Error,
        graph::{edge::Proof, Edge, Vertex},
        graph::{new_connection_pool, new_db_connection},
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
        // let db = new_db_connection().await?;
        // let created = Identity::create_dummy(&db).await?;
        // println!("display_name={}", created.display_name);
        // let raw_db = new_raw_db_connection().await?;
        let pool = new_connection_pool().await;
        let found = Identity::find_by_display_name(&pool, String::from("some created.display"))
            .await?
            .expect("Record not found");
        println!("{:#?}", found);
        Ok(())
    }

    #[tokio::test]
    async fn test_find_by_platforms_identity() -> Result<(), Error> {
        let pool = new_connection_pool().await;
        let identities = Identity::find_by_platforms_identity(
            &pool,
            &vec![Platform::Ethereum, Platform::Twitter],
            "0x00000003cd3aa7e760877f03275621d2692f5841",
        )
        .await
        .unwrap();
        println!("{:?}", identities);
        Ok(())
    }

    #[tokio::test]
    async fn test_neighbors() -> Result<(), Error> {
        let db = new_db_connection().await?;
        let pool = new_connection_pool().await;
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
        let neighbors = id1.neighbors(&pool, 2, None).await?;
        assert_eq!(3, neighbors.len());
        // assert!(neighbors
        //     .iter()
        //     .all(|i| i.uuid == id2.uuid || i.uuid == id3.uuid));
        Ok(())
    }

    #[tokio::test]
    async fn test_neighbors_with_traversal() -> Result<(), Error> {
        let pool = new_connection_pool().await;
        let found = Identity::find_by_display_name(&pool, String::from("Kc37j5zNLG5RLxbWGOz"))
            .await?
            .expect("Record not found");
        println!("{:#?}", found);
        let neighbors = found
            .neighbors_with_traversal(&pool, 3, None)
            .await
            .unwrap();
        println!("{:#?}", neighbors);
        Ok(())
    }

    #[tokio::test]
    async fn test_string_to_platfrom() -> Result<(), Error> {
        let platforms = vec![
            String::from("github"),
            String::from("twitter"),
            // String::from("aaa"),
        ];
        let platform_list = crate::controller::vec_string_to_vec_platform(platforms)?;
        println!("{:?}", platform_list);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_identities() -> Result<(), Error> {
        let pool = new_connection_pool().await;
        let ids = vec![
            String::from("2NOea6D9n8T8fQf464L"),
            String::from("lJTcEp2"),
            String::from("fake"),
        ];
        let result = get_identities(&pool, ids).await;
        println!("{:#?}", result);
        Ok(())
    }
}
