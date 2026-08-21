use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
pub struct Software {
    pub id: String,
    pub url: String,
    #[serde(rename = "catalogId")]
    pub catalog_id: Option<String>,
    #[serde(rename = "publiccodeYml", default)]
    pub publiccode_yml: Option<String>,
}

pub struct CatalogClient {
    client: Client,
    base: String,
    token: String,
}

impl CatalogClient {
    pub fn new(client: Client, base: String, token: String) -> Self {
        Self {
            client,
            base,
            token,
        }
    }

    pub async fn list_software(&self, catalog: Option<&str>) -> Result<Vec<Software>> {
        let mut out = Vec::new();
        let mut url = match catalog {
            Some(id) => format!("{}/catalogs/{}/software?page[size]=100", self.base, id),
            None => format!("{}/software?page[size]=100", self.base),
        };
        loop {
            let resp = self.client.get(&url).send().await?;
            if !resp.status().is_success() {
                return Err(anyhow!("list software returned {}", resp.status()));
            }
            let page: Value = resp.json().await?;
            if let Some(data) = page["data"].as_array() {
                for item in data {
                    out.push(serde_json::from_value(item.clone())?);
                }
            }
            match page["links"]["next"].as_str() {
                Some(next) if !next.is_empty() => {
                    url = Url::parse(&url)
                        .and_then(|b| b.join(next))
                        .map(|u| u.to_string())
                        .map_err(|e| anyhow!("bad next link: {e}"))?;
                }
                _ => break,
            }
        }
        Ok(out)
    }

    pub async fn resolve_root_catalog_id(&self) -> Result<String> {
        let resp = self
            .client
            .get(format!("{}/catalogs?all=true", self.base))
            .send()
            .await?;
        let page: Value = resp.json().await?;
        let data = page["data"].as_array().cloned().unwrap_or_default();
        for c in &data {
            if c["alternativeId"].as_str() == Some("\u{2205}") {
                return Ok(c["id"].as_str().unwrap_or_default().to_string());
            }
        }
        Err(anyhow!("no root catalog (alternativeId=∅) found"))
    }

    pub async fn get_software_analysis(&self, id: &str) -> Result<Value> {
        let url = format!("{}/software/{}/analysis", self.base, id);
        let resp = self.client.get(&url).bearer_auth(&self.token).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("get {} returned {}", url, resp.status()));
        }
        Ok(resp.json::<Value>().await?)
    }

    pub async fn patch_software_analysis(&self, id: &str, body: &Value) -> Result<()> {
        self.patch(&format!("{}/software/{}/analysis", self.base, id), body)
            .await
    }

    pub async fn patch_catalog_analysis(&self, id: &str, body: &Value) -> Result<()> {
        self.patch(&format!("{}/catalogs/{}/analysis", self.base, id), body)
            .await
    }

    async fn patch(&self, url: &str, body: &Value) -> Result<()> {
        let resp = self
            .client
            .patch(url)
            .bearer_auth(&self.token)
            .header("Content-Type", "application/merge-patch+json")
            .json(body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!("patch {} returned {}", url, resp.status()));
        }
        Ok(())
    }
}
