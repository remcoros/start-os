use std::collections::BTreeMap;
use std::path::Path;

use clap::builder::ValueParserFactory;
use clap::Parser;
use color_eyre::eyre::eyre;
use imbl_value::InternedString;
use rpc_toolkit::{command, from_fn_async, AnyContext, Empty, HandlerExt, ParentHandler};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::context::{CliContext, RpcContext};
use crate::prelude::*;
use crate::util::clap::FromStrParser;
use crate::util::serde::{display_serializable, HandlerExtSerde, WithIoFormat};

static SSH_AUTHORIZED_KEYS_FILE: &str = "/home/start9/.ssh/authorized_keys";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SshKeys(BTreeMap<InternedString, WithTimeData<SshPubKey>>);
impl SshKeys {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }
}
impl Map for SshKeys {
    type Key = InternedString;
    type Value = WithTimeData<SshPubKey>;
    fn key_str(key: &Self::Key) -> Result<impl AsRef<str>, Error> {
        Ok(key)
    }
    fn key_string(key: &Self::Key) -> Result<InternedString, Error> {
        Ok(key.clone())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SshPubKey(
    #[serde(serialize_with = "crate::util::serde::serialize_display")]
    #[serde(deserialize_with = "crate::util::serde::deserialize_from_str")]
    openssh_keys::PublicKey,
);
impl ValueParserFactory for SshPubKey {
    type Parser = FromStrParser<Self>;
    fn value_parser() -> Self::Parser {
        FromStrParser::new()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SshKeyResponse {
    pub alg: String,
    pub fingerprint: InternedString,
    pub hostname: String,
    pub created_at: String,
}
impl std::fmt::Display for SshKeyResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "{} {} {} {}",
            self.created_at, self.alg, self.fingerprint, self.hostname
        )
    }
}

impl std::str::FromStr for SshPubKey {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(|pk| SshPubKey(pk)).map_err(|e| Error {
            source: e.into(),
            kind: crate::ErrorKind::ParseSshKey,
            revision: None,
        })
    }
}

// #[command(subcommands(add, delete, list,))]
pub fn ssh() -> ParentHandler {
    ParentHandler::new()
        .subcommand(
            "add",
            from_fn_async(add)
                .no_display()
                .with_remote_cli::<CliContext>(),
        )
        .subcommand(
            "delete",
            from_fn_async(delete)
                .no_display()
                .with_remote_cli::<CliContext>(),
        )
        .subcommand(
            "list",
            from_fn_async(list)
                .with_display_serializable()
                .with_custom_display_fn::<AnyContext, _>(|handle, result| {
                    Ok(display_all_ssh_keys(handle.params, result))
                })
                .with_remote_cli::<CliContext>(),
        )
}

#[derive(Deserialize, Serialize, Parser)]
#[serde(rename_all = "kebab-case")]
#[command(rename_all = "kebab-case")]
pub struct AddParams {
    key: SshPubKey,
}

#[instrument(skip_all)]
pub async fn add(ctx: RpcContext, AddParams { key }: AddParams) -> Result<SshKeyResponse, Error> {
    let mut key = WithTimeData::new(key);
    let fingerprint = InternedString::intern(key.0.fingerprint_md5());
    ctx.db
        .mutate(move |m| {
            m.as_private_mut()
                .as_ssh_pubkeys_mut()
                .insert(&fingerprint, &key)?;

            Ok(SshKeyResponse {
                alg: key.0.keytype().to_owned(),
                fingerprint,
                hostname: key.0.comment.take().unwrap_or_default(),
                created_at: key.created_at.to_rfc3339(),
            })
        })
        .await
}

#[derive(Deserialize, Serialize, Parser)]
#[serde(rename_all = "kebab-case")]
#[command(rename_all = "kebab-case")]
pub struct DeleteParams {
    fingerprint: InternedString,
}

#[instrument(skip_all)]
pub async fn delete(
    ctx: RpcContext,
    DeleteParams { fingerprint }: DeleteParams,
) -> Result<(), Error> {
    let keys = ctx
        .db
        .mutate(|m| {
            let keys_ref = m.as_private_mut().as_ssh_pubkeys_mut();
            if keys_ref.remove(&fingerprint)?.is_some() {
                keys_ref.de()
            } else {
                Err(Error {
                    source: color_eyre::eyre::eyre!("SSH Key Not Found"),
                    kind: crate::error::ErrorKind::NotFound,
                    revision: None,
                })
            }
        })
        .await?;
    sync_keys(&keys, SSH_AUTHORIZED_KEYS_FILE).await
}

fn display_all_ssh_keys(params: WithIoFormat<Empty>, result: Vec<SshKeyResponse>) {
    use prettytable::*;

    if let Some(format) = params.format {
        return display_serializable(format, params);
    }

    let mut table = Table::new();
    table.add_row(row![bc =>
        "CREATED AT",
        "ALGORITHM",
        "FINGERPRINT",
        "HOSTNAME",
    ]);
    for key in result {
        let row = row![
            &format!("{}", key.created_at),
            &key.alg,
            &key.fingerprint,
            &key.hostname,
        ];
        table.add_row(row);
    }
    table.print_tty(false).unwrap();
}

#[instrument(skip_all)]
pub async fn list(ctx: RpcContext) -> Result<Vec<SshKeyResponse>, Error> {
    ctx.db
        .peek()
        .await
        .into_private()
        .into_ssh_pubkeys()
        .into_entries()?
        .into_iter()
        .map(|(fingerprint, key)| {
            let mut key = key.de()?;
            Ok(SshKeyResponse {
                alg: key.0.keytype().to_owned(),
                fingerprint,
                hostname: key.0.comment.take().unwrap_or_default(),
                created_at: key.created_at.to_rfc3339(),
            })
        })
        .collect()
}

#[instrument(skip_all)]
pub async fn sync_keys<P: AsRef<Path>>(keys: &SshKeys, dest: P) -> Result<(), Error> {
    use tokio::io::AsyncWriteExt;

    let dest = dest.as_ref();
    let ssh_dir = dest.parent().ok_or_else(|| {
        Error::new(
            eyre!("SSH Key File cannot be \"/\""),
            crate::ErrorKind::Filesystem,
        )
    })?;
    if tokio::fs::metadata(ssh_dir).await.is_err() {
        tokio::fs::create_dir_all(ssh_dir).await?;
    }
    let mut f = tokio::fs::File::create(dest).await?;
    for key in keys.0.values() {
        f.write_all(key.0.to_key_format().as_bytes()).await?;
        f.write_all(b"\n").await?;
    }
    Ok(())
}
