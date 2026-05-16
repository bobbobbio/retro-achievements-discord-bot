// Copyright 2026 Remi Bernotavicius

use anyhow::{bail, Context as _, Result};
use clap::Parser;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
struct Settings {
    retro_achievements_user: String,
    retro_achievements_token: String,
    discord_web_hook_url: String,
}

/// Bot that posts retro achievement posts to discord
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    settings_path: PathBuf,
}

async fn read_toml_file<TypeT: DeserializeOwned>(name: &str, path: &Path) -> Result<TypeT> {
    let toml_str = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read {name} file from: {}", path.display()))?;
    let parsed: TypeT = toml::from_str(&toml_str).with_context(|| "failed to parse {name} file")?;

    Ok(parsed)
}

const RETRO_ACHIEVEMENTS_API_ENDPOINT: &str = "https://retroachievements.org/API";

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct Achievement {
    #[serde(rename = "ID")]
    id: u32,
    title: String,
    description: String,
    points: u32,
    badge_name: String,
    #[serde(rename = "BadgeURL")]
    badge_url: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct Game {
    #[serde(rename = "ID")]
    id: u32,
    title: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct Console {
    #[serde(rename = "ID")]
    id: u32,
    title: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct GetAchievementOfTheWeekResponse {
    achievement: Achievement,
    game: Game,
    console: Console,
}

struct RetroAchievementApi {
    client: reqwest::Client,
    user: String,
    token: String,
}

impl RetroAchievementApi {
    fn new(client: reqwest::Client, user: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            client,
            user: user.into(),
            token: token.into(),
        }
    }

    async fn do_request<ResponseT: DeserializeOwned>(&self, method: &str) -> Result<ResponseT> {
        let url = format!("{RETRO_ACHIEVEMENTS_API_ENDPOINT}/API_{method}.php");
        let response = self
            .client
            .get(&url)
            .basic_auth(&self.user, Some(&self.token))
            .send()
            .await
            .context("error querying retro achievements API")?;

        if !response.status().is_success() {
            bail!(
                "got error from retro achievements API: {}: {}",
                response.status(),
                response
                    .text()
                    .await
                    .unwrap_or_else(|e| format!("<error reading body: {e}>"))
            );
        }

        Ok(response
            .json()
            .await
            .context("got error reading body from retro achievement API")?)
    }
}

struct DiscordClient {
    client: reqwest::Client,
    web_hook: String,
}

impl DiscordClient {
    fn new(client: reqwest::Client, web_hook: impl Into<String>) -> Self {
        Self {
            client,
            web_hook: web_hook.into(),
        }
    }

    async fn post(&self, user: &str, content: &str) -> Result<()> {
        let mut params = HashMap::new();
        params.insert("username", user);
        params.insert("content", content);
        self.client
            .post(&self.web_hook)
            .form(&params)
            .send()
            .await
            .context("got error attempting to post to discord")?;
        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();

    let settings: Settings = read_toml_file("settings", &args.settings_path).await?;

    let http_client = reqwest::Client::new();

    let ra_client = RetroAchievementApi::new(
        http_client.clone(),
        &settings.retro_achievements_user,
        &settings.retro_achievements_token,
    );

    let aotw: GetAchievementOfTheWeekResponse =
        ra_client.do_request("GetAchievementOfTheWeek").await?;

    let discord_client = DiscordClient::new(http_client, &settings.discord_web_hook_url);

    let post = format!(
        "The current AOTW is {}, described as {}, from {} for the {}",
        aotw.achievement.title, aotw.achievement.description, aotw.game.title, aotw.console.title
    );
    discord_client.post("AotwBot", &post).await?;

    Ok(())
}
