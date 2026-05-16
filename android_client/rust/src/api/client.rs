use crate::models::Item;
use flutter_rust_bridge::frb;
use std::sync::RwLock;

#[derive(serde::Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
}

#[frb(opaque)]
pub struct FreshLoopClient {
    base_url: String,
    user_id: RwLock<Option<String>>,
    client: reqwest::Client,
}

impl FreshLoopClient {
    #[frb(sync)]
    pub fn new(base_url: String, user_id: Option<String>) -> Self {
        Self {
            base_url,
            user_id: RwLock::new(user_id),
            client: reqwest::Client::new(),
        }
    }

    #[frb(sync)]
    pub fn set_user_id(&self, user_id: Option<String>) {
        if let Ok(mut lock) = self.user_id.write() {
            *lock = user_id;
        }
    }

    pub async fn login(&self, username: String, password: String) -> anyhow::Result<User> {
        let resp = self.client.post(format!("{}/api/auth/login", self.base_url))
            .json(&serde_json::json!({ "username": username, "password": password }))
            .send()
            .await?;
            
        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("Authentication failed (HTTP {})", resp.status()));
        }
            
        let text = resp.text().await?;
        let user: User = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("Login failed: {} \nResp: {}", e, text))?;
            
        self.set_user_id(Some(user.id.clone()));
        Ok(user)
    }

    pub async fn fetch_items(&self, page: u32, limit: u32) -> anyhow::Result<Vec<Item>> {
        let mut req = self.client.get(format!("{}/api/items?page={}&limit={}", self.base_url, page, limit));
        
        let uid_opt = self.user_id.read().ok().and_then(|lock| lock.clone());
        if let Some(uid) = uid_opt {
            req = req.header("x-user-id", uid);
        }
        
        let resp = req.send().await?;
        let text = resp.text().await?;
        let items: Vec<Item> = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("JSON parse error: {} \nResp: {}", e, text))?;
        
        Ok(items)
    }

    pub async fn mark_as_played(&self, id: String) -> anyhow::Result<()> {
        let uid_opt = self.user_id.read().ok().and_then(|lock| lock.clone());
        if let Some(uid) = uid_opt {
            self.client.post(format!("{}/api/history", self.base_url))
                .header("x-user-id", uid)
                .json(&serde_json::json!({ "item_id": id }))
                .send()
                .await?;
        }
        Ok(())
    }
}
