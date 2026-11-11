//! Repo profiles (spec §16.1): stored in the `profiles` table, matched to a
//! repository when an agent is created (R-002) and read live by the actor for
//! the settings they override. Matching and validation are in
//! `xherd_core::profile`.

use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};
use xherd_core::profile::{self, Profile};

use crate::{ActionError, Daemon};

fn err(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

fn store(e: xherd_store::StoreError) -> ActionError {
    err("store", e.to_string())
}

/// `POST /profiles/import` and the shape `GET /profiles/export` returns
/// (R-1407); no ids, no secrets (profiles hold none).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    pub profiles: Vec<Profile>,
}

impl Daemon {
    pub async fn profiles(&self) -> Result<Vec<Profile>, ActionError> {
        self.store.list_profiles().await.map_err(store)
    }

    pub async fn create_profile(&self, mut p: Profile) -> Result<Profile, ActionError> {
        let all = self.profiles().await?;
        let names: Vec<&str> = all.iter().map(|x| x.name.as_str()).collect();
        profile::check(&mut p, &names).map_err(|m| err("bad_profile", m))?;
        p.id = xherd_core::new_id();
        self.store.put_profile(&p).await.map_err(store)?;
        Ok(p)
    }

    pub async fn update_profile(&self, id: &str, mut p: Profile) -> Result<Profile, ActionError> {
        let all = self.profiles().await?;
        if !all.iter().any(|x| x.id == id) {
            return Err(err("not_found", format!("no profile {id}")));
        }
        let names: Vec<&str> = all
            .iter()
            .filter(|x| x.id != id)
            .map(|x| x.name.as_str())
            .collect();
        profile::check(&mut p, &names).map_err(|m| err("bad_profile", m))?;
        p.id = id.to_string();
        self.store.put_profile(&p).await.map_err(store)?;
        Ok(p)
    }

    /// Agents created on it keep working with the global settings.
    pub async fn delete_profile(&self, id: &str) -> Result<(), ActionError> {
        match self.store.delete_profile(id).await.map_err(store)? {
            true => Ok(()),
            false => Err(err("not_found", format!("no profile {id}"))),
        }
    }

    pub async fn export_profiles(&self) -> Result<Value, ActionError> {
        let mut ps = self.profiles().await?;
        for p in &mut ps {
            p.id.clear();
        }
        Ok(json!({"profiles": ps}))
    }

    /// All or nothing: one invalid profile imports none. A profile whose name
    /// exists replaces that one; the rest are added.
    pub async fn import_profiles(&self, i: Import) -> Result<Value, ActionError> {
        let mut have = self.profiles().await?;
        let (mut added, mut replaced) = (0, 0);
        let mut put = Vec::new();
        for mut p in i.profiles {
            let at = have
                .iter()
                .position(|x| x.name.eq_ignore_ascii_case(p.name.trim()));
            let names: Vec<&str> = have
                .iter()
                .enumerate()
                .filter(|(n, _)| Some(*n) != at)
                .map(|(_, x)| x.name.as_str())
                .collect();
            profile::check(&mut p, &names)
                .map_err(|m| err("bad_profile", format!("{}: {m}", p.name)))?;
            match at {
                Some(n) => {
                    p.id = have[n].id.clone();
                    have[n] = p.clone();
                    replaced += 1;
                }
                None => {
                    p.id = xherd_core::new_id();
                    have.push(p.clone());
                    added += 1;
                }
            }
            put.push(p);
        }
        for p in &put {
            self.store.put_profile(p).await.map_err(store)?;
        }
        Ok(json!({"added": added, "replaced": replaced}))
    }

    /// The profile an agent was created on, if it still exists.
    pub async fn profile_of(&self, a: &xherd_core::Agent) -> Option<Profile> {
        let id = a.profile.as_deref()?;
        self.store
            .list_profiles()
            .await
            .ok()?
            .into_iter()
            .find(|p| p.id == id)
    }

    /// The profile for a repository (R-002): by its `origin` URL, then its
    /// path. `None` when no profile matches.
    pub async fn profile_for_repo(&self, repo: &Path) -> Option<Profile> {
        let all = self.store.list_profiles().await.ok()?;
        if all.is_empty() {
            return None;
        }
        let origin = repo_origin(repo).await;
        let home = xherd_core::runner::agent_env("HOME");
        let path = repo.to_string_lossy();
        profile::resolve(&all, origin.as_deref(), &path, home.as_deref()).cloned()
    }
}

/// The repository's `origin` URL, if it has one.
pub(crate) async fn repo_origin(repo: &Path) -> Option<String> {
    xherd_git::Git::new("git")
        .run(repo, &["config", "--get", "remote.origin.url"])
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
