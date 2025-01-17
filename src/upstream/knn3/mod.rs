#[cfg(test)]
mod tests;

use crate::config::C;
use crate::graph::edge::hold::Hold;
use crate::graph::vertex::{contract::Chain, contract::ContractCategory, Contract};

use crate::upstream::{DataFetcher, DataSource, Fetcher, Platform, Target, TargetProcessedList};
use crate::util::naive_now;
use crate::{
    error::Error,
    graph::{create_identity_to_contract_record, new_db_connection, vertex::Identity},
};

use async_trait::async_trait;
use gql_client::Client;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Deserialize, Debug)]
pub struct EthQueryResponseEns {
    ens: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub struct EthQueryResponse {
    addrs: Vec<EthQueryResponseEns>,
}

#[derive(Serialize)]
pub struct EthQueryVars<'a> {
    addr: &'a str,
}

#[derive(Serialize)]
pub struct ENSQueryVars {
    ens: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub struct EnsQueryResponse {
    addrs: Vec<EnsQueryResponseAddress>,
}

#[derive(Deserialize, Debug)]
pub struct EnsQueryResponseAddress {
    address: String,
}

pub struct Knn3 {}

#[async_trait]
impl Fetcher for Knn3 {
    async fn fetch(target: &Target) -> Result<TargetProcessedList, Error> {
        if !Self::can_fetch(target) {
            return Ok(vec![]);
        }

        match target {
            Target::Identity(_, identity) => fetch_ens_by_eth_wallet(identity).await,
            Target::NFT(_, _, _, id) => fetch_eth_wallet_by_ens(id).await,
        }
    }

    fn can_fetch(_target: &Target) -> bool {
        // TODO: temporarily disable KNN3 fetcher
        false
        // target.in_platform_supported(vec![Platform::Ethereum])
        //     || target.in_nft_supported(vec![ContractCategory::ENS], vec![Chain::Ethereum])
    }
}

/// Use ethereum address to fetch NFTs (especially ENS).
async fn fetch_ens_by_eth_wallet(identity: &str) -> Result<TargetProcessedList, Error> {
    let query = r#"
        query EnsByAddressQuery($addr: String!){
            addrs(where: { address: $addr }) {
                ens
            }
        }
    "#;

    let client = Client::new(C.upstream.knn3_service.url.clone());
    let vars = EthQueryVars {
        addr: &identity.to_lowercase(), // Yes, KNN3 is case-sensitive.
    };

    let resp: Result<Option<EthQueryResponse>, _> = client.query_with_vars(query, vars).await;
    if resp.is_err() {
        warn!(
            "KNN3 fetch | Failed to fetch addrs: {}, err: {:?}",
            identity,
            resp.err()
        );
        return Ok(vec![]);
    }

    let res = resp.unwrap().unwrap();
    if res.addrs.is_empty() {
        info!("KNN3 fetch | address: {} cannot find any result", identity);
        return Ok(vec![]);
    }

    let ens_vec = res.addrs.first().unwrap();
    let db = new_db_connection().await?;

    for ens in ens_vec.ens.iter() {
        let from: Identity = Identity {
            uuid: Some(Uuid::new_v4()),
            platform: Platform::Ethereum,
            identity: identity.to_lowercase(),
            created_at: None,
            // Don't use ETH's wallet as display_name, use ENS reversed lookup instead.
            display_name: None,
            added_at: naive_now(),
            avatar_url: None,
            profile_url: None,
            updated_at: naive_now(),
        };
        let to: Contract = Contract {
            uuid: Uuid::new_v4(),
            category: ContractCategory::ENS,
            address: ContractCategory::ENS.default_contract_address().unwrap(),
            chain: Chain::Ethereum,
            symbol: None,
            updated_at: naive_now(),
        };
        let ownership: Hold = Hold {
            uuid: Uuid::new_v4(),
            transaction: None,
            id: ens.to_string(),
            source: DataSource::Knn3,
            created_at: None,
            updated_at: naive_now(),
            fetcher: DataFetcher::RelationService,
        };
        create_identity_to_contract_record(&db, &from, &to, &ownership).await?;
    }
    Ok(ens_vec
        .ens
        .iter()
        .map(|ens| {
            Target::NFT(
                Chain::Ethereum,
                ContractCategory::ENS,
                ContractCategory::ENS.default_contract_address().unwrap(),
                ens.clone(),
            )
        })
        .collect())
}

async fn fetch_eth_wallet_by_ens(id: &str) -> Result<TargetProcessedList, Error> {
    let query = r#"
        query AddressByENSQuery($ens: [String]){
            addrs(where: { ens: $ens }) {
                address
            }
        }
    "#;
    let client = Client::new(C.upstream.knn3_service.url.clone());
    let vars = ENSQueryVars {
        ens: vec![id.to_string()],
    };
    let response = client
        .query_with_vars::<EnsQueryResponse, _>(query, vars)
        .await;
    if response.is_err() {
        warn!(
            "KNN3 fetch | Failed to fetch addrs using ENS: {}, error: {:?}",
            id,
            response.err()
        );
        return Ok(vec![]);
    }

    let result = response.unwrap().unwrap();
    if result.addrs.is_empty() {
        info!("KNN3 fetch | ENS {} has no result", id);
        return Ok(vec![]);
    }

    // NOTE: not sure if this result must have one and only one.
    let address = result.addrs.first().unwrap().address.clone();
    let from = Identity {
        uuid: Some(Uuid::new_v4()),
        platform: Platform::Ethereum,
        identity: address.to_lowercase(),
        // Don't use ETH's wallet as display_name, use ENS reversed lookup instead.
        display_name: None,
        profile_url: None,
        avatar_url: None,
        created_at: None,
        added_at: naive_now(),
        updated_at: naive_now(),
    };
    let to = Contract {
        uuid: Uuid::new_v4(),
        updated_at: naive_now(),
        category: ContractCategory::ENS,
        address: ContractCategory::ENS.default_contract_address().unwrap(),
        chain: Chain::Ethereum,
        symbol: None,
    };
    let hold = Hold {
        uuid: Uuid::new_v4(),
        transaction: None,
        id: id.into(),
        source: DataSource::Knn3,
        created_at: None,
        updated_at: naive_now(),
        fetcher: DataFetcher::RelationService,
    };
    let db = new_db_connection().await?;
    create_identity_to_contract_record(&db, &from, &to, &hold).await?;

    Ok(vec![Target::Identity(Platform::Ethereum, address)])
}
