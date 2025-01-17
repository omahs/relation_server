#[cfg(test)]
mod tests;

use crate::config::C;
use crate::error::Error;
use crate::graph::create_identity_to_identity_record;
use crate::graph::{edge::Proof, new_db_connection, vertex::Identity};
use crate::upstream::{DataSource, Fetcher, Platform, TargetProcessedList};
use crate::util::{make_client, naive_now, parse_body};
use async_trait::async_trait;
use serde::Deserialize;

use std::str::FromStr;
use uuid::Uuid;

use super::{DataFetcher, Target};

#[derive(Deserialize, Debug)]
pub struct KeybaseResponse {
    pub status: Status,
    pub them: Vec<PersonInfo>,
}

#[derive(Deserialize, Debug)]
pub struct PersonInfo {
    pub id: String,
    pub basics: Basics,
    pub proofs_summary: ProofsSummary,
}

#[derive(Deserialize, Debug)]
pub struct Status {
    pub code: i32,
    pub name: String,
}

#[derive(Deserialize, Debug)]
pub struct ProofsSummary {
    pub all: Vec<ProofItem>,
}

#[derive(Deserialize, Debug)]
pub struct Basics {
    pub username: String,
    pub ctime: i64,
    pub mtime: i64,
    pub id_version: i32,
    pub track_version: i32,
    pub last_id_change: i64,
    pub username_cased: String,
    pub status: i32,
    pub salt: String,
    pub eldest_seqno: i32,
}

#[derive(Deserialize, Debug)]
pub struct ProofItem {
    pub proof_type: String,
    pub nametag: String,
    pub state: i32,
    pub service_url: String,
    pub proof_url: String,
    pub sig_id: String,
    pub proof_id: String,
    pub human_url: String,
    pub presentation_group: String,
    pub presentation_tag: String,
}

#[derive(Deserialize, Debug)]
pub struct ErrorResponse {
    pub message: String,
}

#[derive(Default)]
pub struct Keybase {}

#[async_trait]
impl Fetcher for Keybase {
    async fn fetch(target: &Target) -> Result<TargetProcessedList, Error> {
        if !Self::can_fetch(target) {
            return Ok(vec![]);
        }

        match target {
            Target::Identity(platform, identity) => {
                fetch_connections_by_platform_identity(platform, identity).await
            }
            Target::NFT(_, _, _, _) => todo!(),
        }
    }

    fn can_fetch(target: &Target) -> bool {
        target.in_platform_supported(vec![Platform::Twitter, Platform::Github, Platform::Reddit])
    }
}

async fn fetch_connections_by_platform_identity(
    platform: &Platform,
    identity: &str,
) -> Result<TargetProcessedList, Error> {
    let client = make_client();
    let uri: http::Uri = match format!(
        "{}?{}={}&fields=proofs_summary",
        C.upstream.keybase_service.url, platform, identity
    )
    .parse()
    {
        Ok(n) => n,
        Err(err) => return Err(Error::ParamError(format!("Uri format Error: {}", err))),
    };

    let mut resp = client.get(uri).await?;
    if !resp.status().is_success() {
        let body: ErrorResponse = parse_body(&mut resp).await?;
        return Err(Error::General(
            format!("Keybase Result Get Error: {}", body.message),
            resp.status(),
        ));
    }

    let mut body: KeybaseResponse = parse_body(&mut resp).await?;
    if body.status.code != 0 {
        return Err(Error::General(
            format!("Keybase Result Get Error: {}", body.status.name),
            resp.status(),
        ));
    }

    let person_info = match body.them.pop() {
        Some(i) => i,
        None => {
            return Err(Error::NoResult);
        }
    };
    let user_id = person_info.id;
    let user_name = person_info.basics.username;
    let db = new_db_connection().await?;
    let mut next_targets: TargetProcessedList = Vec::new();

    for p in person_info.proofs_summary.all.into_iter() {
        let from: Identity = Identity {
            uuid: Some(Uuid::new_v4()),
            platform: Platform::Keybase,
            identity: user_id.clone(),
            created_at: None,
            display_name: Some(user_name.clone()),
            added_at: naive_now(),
            avatar_url: None,
            profile_url: None,
            updated_at: naive_now(),
        };

        if Platform::from_str(p.proof_type.as_str()).is_err() {
            continue;
        }
        let to: Identity = Identity {
            uuid: Some(Uuid::new_v4()),
            platform: Platform::from_str(p.proof_type.as_str()).unwrap(),
            identity: p.nametag.clone().to_lowercase(),
            created_at: None,
            display_name: Some(p.nametag.clone()),
            added_at: naive_now(),
            avatar_url: None,
            profile_url: None,
            updated_at: naive_now(),
        };

        let pf: Proof = Proof {
            uuid: Uuid::new_v4(),
            source: DataSource::Keybase,
            record_id: Some(p.proof_id.clone()),
            created_at: None,
            updated_at: naive_now(),
            fetcher: DataFetcher::RelationService,
        };

        create_identity_to_identity_record(&db, &from, &to, &pf).await?;

        next_targets.push(Target::Identity(
            Platform::from_str(&p.proof_type).unwrap(),
            p.nametag,
        ));
    }

    Ok(next_targets)
}
