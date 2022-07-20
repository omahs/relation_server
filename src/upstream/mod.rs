mod aggregation;
mod keybase;
mod knn3;
mod proof_client;
mod rss3;
mod sybil_list;

use crate::{
    error::Error,
    graph::vertex::contract::{Chain, ContractCategory},
    upstream::{
        aggregation::Aggregation, keybase::Keybase, knn3::Knn3, proof_client::ProofClient,
        rss3::Rss3, sybil_list::SybilList,
    },
};
use async_trait::async_trait;
use futures::future::join_all;
use http::StatusCode;
use log::{debug, info, trace};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumIter, EnumString};

/// List when processing identities.
type TargetProcessedList = Vec<Target>;

/// Target to fetch.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Identity with given platform and identity.
    Identity(Platform, String),

    /// NFT with given chain, category and contract_address, NFT_ID.
    NFT(Chain, ContractCategory, String, String),
}
impl Default for Target {
    fn default() -> Self {
        Target::Identity(Platform::default(), "".to_string())
    }
}
impl Target {
    /// Judge if this target is in supported platforms list given by upstream.
    pub fn in_platform_supported(&self, platforms: Vec<Platform>) -> bool {
        match self {
            Self::NFT(_, _, _, _) => false,
            Self::Identity(platform, _) => platforms.contains(platform),
        }
    }

    /// Judge if this target is in supported NFT category / chain list given by upstream.
    pub fn in_nft_supported(
        &self,
        nft_categories: Vec<ContractCategory>,
        nft_chains: Vec<Chain>,
    ) -> bool {
        match self {
            Self::Identity(_, _) => false,
            Self::NFT(chain, category, _, _) => {
                nft_categories.contains(category) && nft_chains.contains(chain)
            }
        }
    }

    pub fn platform(&self) -> Result<Platform, Error> {
        match self {
            Self::Identity(platform, _) => Ok(platform.clone()),
            Self::NFT(_, _, _, _) => Err(Error::General(
                "Target: Get platform error: Not an Identity".into(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )),
        }
    }

    pub fn identity(&self) -> Result<String, Error> {
        match self {
            Self::Identity(_, identity) => Ok(identity.clone()),
            Self::NFT(_, _, _, _) => Err(Error::General(
                "Target: Get identity error: Not an Identity".into(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )),
        }
    }

    pub fn nft_chain(&self) -> Result<Chain, Error> {
        match self {
            Self::Identity(_, _) => Err(Error::General(
                "Target: Get nft chain error: Not an NFT".into(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )),
            Self::NFT(chain, _, _, _) => Ok(chain.clone()),
        }
    }

    pub fn nft_category(&self) -> Result<ContractCategory, Error> {
        match self {
            Self::Identity(_, _) => Err(Error::General(
                "Target: Get nft category error: Not an NFT".into(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )),
            Self::NFT(_, category, _, _) => Ok(category.clone()),
        }
    }

    pub fn nft_id(&self) -> Result<String, Error> {
        match self {
            Self::Identity(_, _) => Err(Error::General(
                "Target: Get nft id error: Not an NFT".into(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )),
            Self::NFT(_, _, _, nft_id) => Ok(nft_id.clone()),
        }
    }
}
impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identity(platform, identity) => write!(f, "Identity/{}/{}", platform, identity),
            Self::NFT(chain, category, address, nft_id) => {
                write!(f, "NFT/{}/{}/{}/{}", chain, category, address, nft_id)
            }
        }
    }
}

/// All identity platform.
/// TODO: move this definition into `graph/vertex/identity`, since it is not specific to upstream.
#[derive(
    Serialize, Deserialize, Debug, EnumString, Clone, Display, PartialEq, EnumIter, Default,
)]
pub enum Platform {
    /// Twitter
    #[strum(serialize = "twitter")]
    #[serde(rename = "twitter")]
    #[default]
    Twitter,

    /// Ethereum wallet `0x[a-f0-9]{40}`
    #[strum(serialize = "ethereum", serialize = "eth")]
    #[serde(rename = "ethereum")]
    Ethereum,

    /// NextID
    #[strum(serialize = "nextid")]
    #[serde(rename = "nextid")]
    NextID,

    /// Keybase
    #[strum(serialize = "keybase")]
    #[serde(rename = "keybase")]
    Keybase,

    /// Github
    #[strum(serialize = "github")]
    #[serde(rename = "github")]
    Github,

    /// Unknown
    #[strum(serialize = "unknown")]
    #[serde(rename = "unknown")]
    Unknown,
}

/// All data respource platform.
#[derive(
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Display,
    EnumString,
    PartialEq,
    Eq,
    EnumIter,
    Default,
    Copy,
    async_graphql::Enum,
)]
pub enum DataSource {
    /// https://github.com/Uniswap/sybil-list/blob/master/verified.json
    #[strum(serialize = "sybil")]
    #[serde(rename = "sybil")]
    #[graphql(name = "sybil")]
    SybilList,

    /// https://keybase.io/docs/api/1.0/call/user/lookup
    #[strum(serialize = "keybase")]
    #[serde(rename = "keybase")]
    #[graphql(name = "keybase")]
    Keybase,

    /// https://docs.next.id/docs/proof-service/api
    #[strum(serialize = "nextid")]
    #[serde(rename = "nextid")]
    #[graphql(name = "nextid")]
    NextID, // = "nextID",

    /// https://rss3.io/network/api.html
    #[strum(serialize = "rss3")]
    #[serde(rename = "rss3")]
    #[graphql(name = "rss3")]
    Rss3, // = "rss3",

    /// https://docs.knn3.xyz/graphql/
    #[strum(serialize = "knn3")]
    #[serde(rename = "knn3")]
    #[graphql(name = "knn3")]
    Knn3, // = "rss3",

    #[strum(serialize = "cyberconnect")]
    #[serde(rename = "cyberconnect")]
    #[graphql(name = "cyberconnect")]
    CyberConnect,

    #[strum(serialize = "ethLeaderboard")]
    #[serde(rename = "ethLeaderboard")]
    #[graphql(name = "ethLeaderboard")]
    EthLeaderboard,

    /// Unknown
    #[strum(serialize = "unknown")]
    #[serde(rename = "unknown")]
    #[graphql(name = "unknown")]
    #[default]
    Unknown,
}

/// All asymmetric cryptography algorithm supported by RelationService.
#[derive(Serialize, Deserialize)]
pub enum Algorithm {
    EllipticCurve,
}

/// All elliptic curve supported by RelationService.
#[derive(Serialize, Deserialize)]
pub enum Curve {
    Secp256K1,
}

/// Fetcher defines how to fetch data from upstream.
#[async_trait]
pub trait Fetcher {
    /// Fetch data from given source.
    async fn fetch(target: &Target) -> Result<TargetProcessedList, Error>;

    /// Determine if this upstream can fetch this target.
    fn can_fetch(target: &Target) -> bool;
}

/// Find all available (platform, identity) in all `Upstream`s.
pub async fn fetch_all(initial_target: Target) -> Result<(), Error> {
    info!("fetch_all : {}", initial_target);
    let mut up_next: TargetProcessedList = vec![initial_target];
    let mut processed: TargetProcessedList = vec![];
    while up_next.len() > 0 {
        debug!("fetch_all::up_next | {:?}", up_next);
        let target = up_next.pop().unwrap();
        let fetched = fetch_one(&target).await?;
        processed.push(target.clone());
        fetched.into_iter().for_each(|f| {
            if processed.contains(&f) {
                trace!("fetch_all::iter | Fetched {} | duplicated", f);
            } else {
                trace!("fetch_all::iter | Fetched {} | pushed into up_next", f);
                up_next.push(f.clone());
            }
        });
    }

    Ok(())
}

// async fn fetching(source: Upstream, platform: &Platform, identity: &str) -> TargetProcessedList {
//     let fetcher = source.get_fetcher(platform, &identity);
//     let ability = fetcher.ability();
//     let mut res: TargetProcessedList = Vec::new();
//     for (platforms, _) in ability.into_iter() {
//         if platforms.iter().any(|i| i == platform) {
//             debug!(
//                 "fetch_one | Fetching {} / {} from {:?}",
//                 platform, identity, source
//             );
//             match fetcher.fetch().await {
//                 Ok(resp) => {
//                     debug!(
//                         "fetch_one | Fetched ({} / {} from {:?}): {:?}",
//                         platform, identity, source, resp
//                     );
//                     res.extend(resp);
//                 }
//                 Err(err) => {
//                     warn!(
//                         "fetch_one | Failed to fetch ({} / {} from {:?}): {:?}",
//                         platform, identity, source, err
//                     );
//                     continue;
//                 }
//             };
//         }
//     }
//     res
// }

/// Find one (platform, identity) pair in all upstreams.
/// Returns identities just fetched for next iter..
pub async fn fetch_one(target: &Target) -> Result<TargetProcessedList, Error> {
    // FIXME: Yeah yeah I know it's stupid.
    let results: TargetProcessedList = join_all(vec![
        Aggregation::fetch(target),
        SybilList::fetch(target),
        Keybase::fetch(target),
        ProofClient::fetch(target),
        Rss3::fetch(target),
        Knn3::fetch(target),
    ])
    .await
    .into_iter()
    .flat_map(|res| res.unwrap_or(vec![]))
    .collect();

    Ok(results)
}

/// Prefetch all prefetchable upstreams, e.g. SybilList.
pub async fn prefetch() -> Result<(), Error> {
    info!("Prefetching sybil_list ...");
    sybil_list::prefetch().await?;
    info!("Prefetch completed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::error::Error;
    use crate::upstream::{fetch_all, fetch_one, Platform, Target};

    #[tokio::test]
    async fn test_fetcher_result() -> Result<(), Error> {
        let result = fetch_all(Target::Identity(Platform::Twitter, "0xsannie".into())).await?;
        assert_eq!(result, ());

        Ok(())
    }

    #[tokio::test]
    async fn test_fetch_one_result() -> Result<(), Error> {
        let result = fetch_one(&Target::Identity(Platform::Twitter, "0xsannie".into())).await?;
        assert_ne!(result.len(), 0);

        Ok(())
    }
}
