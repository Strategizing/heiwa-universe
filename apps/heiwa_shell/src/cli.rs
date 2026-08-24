use anyhow::Result;

use crate::cmd;

pub async fn try_handle(args: &[String]) -> Result<bool> {
    match args.get(1).map(String::as_str) {
        Some("cost") => {
            cmd::cost::run(&args[2..])?;
            Ok(true)
        }
        Some("life") => {
            cmd::life::run(&args[2..])?;
            Ok(true)
        }
        Some("goal") => {
            cmd::goal::run(&args[2..])?;
            Ok(true)
        }
        Some("compress") => {
            cmd::compress::run(&args[2..]).await?;
            Ok(true)
        }
        Some("app") => {
            cmd::app::run(&args[2..]).await?;
            Ok(true)
        }
        Some("capabilities") => {
            cmd::capabilities::run(&args[2..])?;
            Ok(true)
        }
        Some("workers") => {
            cmd::workers::run(&args[2..])?;
            Ok(true)
        }
        Some("approvals") => {
            cmd::approvals::run(&args[2..])?;
            Ok(true)
        }
        Some("auto") | Some("automations") => {
            cmd::auto::run(&args[2..]).await?;
            Ok(true)
        }
        Some("mesh") => {
            cmd::mesh::run(&args[2..])?;
            Ok(true)
        }
        Some("mail") => {
            cmd::mail::run(&args[2..]).await?;
            Ok(true)
        }
        Some("schedule") => {
            cmd::schedule::run(&args[2..])?;
            Ok(true)
        }
        Some("calendar") => {
            cmd::calendar::run(&args[2..]).await?;
            Ok(true)
        }
        Some("connect") => {
            cmd::connectors::run(&args[2..]).await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}
