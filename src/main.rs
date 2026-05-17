// Copyright 2026 Remi Bernotavicius

use anyhow::{bail, Context as _, Result};
use chrono::{DateTime, Local, Utc};
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
const RETRO_ACHIEVEMENTS_MEDIA_URL: &str = "https://media.retroachievements.org";
const RETRO_ACHIEVEMENTS_WEB_URL: &str = "https://retroachievements.org";

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
struct GameRef {
    #[serde(rename = "ID")]
    id: u32,
    title: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct Game {
    title: String,
    game_title: String,
    #[serde(rename = "ConsoleID")]
    console_id: u32,
    console_name: String,
    game_icon: String,
    image_icon: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct ConsoleRef {
    #[serde(rename = "ID")]
    id: u32,
    title: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct GetAchievementOfTheWeekResponse {
    achievement: Achievement,
    game: GameRef,
    console: ConsoleRef,
    start_at: DateTime<Utc>,
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

    async fn do_request<ResponseT: DeserializeOwned>(
        &self,
        method: &str,
        params: Vec<(String, String)>,
    ) -> Result<ResponseT> {
        let url = format!("{RETRO_ACHIEVEMENTS_API_ENDPOINT}/API_{method}.php");
        let response = self
            .client
            .get(&url)
            .query(&params)
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

    async fn get_achievement_of_the_week(&self) -> Result<GetAchievementOfTheWeekResponse> {
        self.do_request("GetAchievementOfTheWeek", vec![]).await
    }

    async fn get_game(&self, game: &GameRef) -> Result<Game> {
        self.do_request("GetGame", vec![("i".into(), game.id.to_string())])
            .await
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

    async fn post(&self, user: &str, content: &str, embeds: &serde_json::Value) -> Result<()> {
        let payload = serde_json::json! ({
            "username": &user,
            "content": &content,
            "embeds": &embeds
        });
        let mut params = HashMap::new();
        params.insert("payload_json", serde_json::to_string(&payload)?);
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

    let aotw = ra_client.get_achievement_of_the_week().await?;
    let game = ra_client.get_game(&aotw.game).await?;

    let discord_client = DiscordClient::new(http_client, &settings.discord_web_hook_url);

    let embeds = serde_json::json! ([
        {
            "title": &aotw.achievement.title,
            "image": {
                "url": format!(
                    "{RETRO_ACHIEVEMENTS_MEDIA_URL}{}",
                    aotw.achievement.badge_url
                )
            },
            "fields": [
                { "name": "", "value": &aotw.achievement.description },
            ],
            "url": format!("{RETRO_ACHIEVEMENTS_WEB_URL}/achievement/{}", aotw.achievement.id)
        },
        {
            "title": &aotw.game.title,
            "image": {
                "url": format!(
                    "{RETRO_ACHIEVEMENTS_MEDIA_URL}{}",
                    game.game_icon
                )
            },
            "fields": [
                { "name": "", "value": &aotw.console.title },
            ],
            "url": format!("{RETRO_ACHIEVEMENTS_WEB_URL}/game/{}", aotw.game.id)
        },
    ]);
    let post = format!(
        "**Achievement of the Week for {}**",
        aotw.start_at.with_timezone(&Local).format("%d/%m/%Y")
    );
    discord_client.post("AotwBot", &post, &embeds).await?;

    Ok(())
}
