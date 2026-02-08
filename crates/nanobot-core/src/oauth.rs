use crate::config::{OAuthTokens, ProviderToken};
use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng;
use sha2::{Digest, Sha256};

pub struct OAuthFlow {
    provider: String,
    code_verifier: Option<String>,
}

impl OAuthFlow {
    pub fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_string(),
            code_verifier: None,
        }
    }

    /// Generate the OAuth login URL for the user to open in browser
    /// This now implements PKCE (S256) for Antigravity
    pub fn get_auth_url(&mut self) -> Result<String> {
        match self.provider.as_str() {
            "antigravity" => {
                // PKCE Implementation
                // 1. Generate 32 random bytes and hex-encode them for the verifier
                let mut rng = rand::rng();
                let random_bytes: [u8; 32] = rng.random();
                let verifier = hex_encode(&random_bytes); // 64 chars

                // 2. Store verifier in struct for later exchange step
                self.code_verifier = Some(verifier.clone());

                // 3. Compute Code Challenge: Base64Url(SHA256(verifier))
                let mut hasher = Sha256::new();
                hasher.update(verifier.as_bytes());
                let result = hasher.finalize();
                let challenge = URL_SAFE_NO_PAD.encode(result);

                // 4. Construct URL
                let client_id =
                    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
                let redirect_uri = "http://localhost:51121/oauth-callback";
                let scope = "openid email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";
                let response_type = "code";
                let access_type = "offline";
                let prompt = "consent";

                // State serves double duty: CSRF protection and PKCE verifier tracking (simple mechanism)
                // We use the verifier itself as state to ensure we are matching the correct flow
                let state = verifier;

                Ok(format!(
                    "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type={}&scope={}&access_type={}&prompt={}&code_challenge={code_challenge}&code_challenge_method=S256&state={state}",
                    client_id,
                    urlencoding::encode(redirect_uri),
                    response_type,
                    urlencoding::encode(scope),
                    access_type,
                    prompt,
                    code_challenge = challenge,
                    state = state
                ))
            }
            "openai" => {
                // OpenAI OAuth (requires official client ID - not publicly available)
                let client_id = "YOUR_OPENAI_CLIENT_ID";
                let redirect_uri = "http://localhost:8080/callback";
                let scope = "openid email";

                Ok(format!(
                    "https://auth.openai.com/authorize?client_id={}&redirect_uri={}&response_type=code&scope={}",
                    client_id,
                    urlencoding::encode(redirect_uri),
                    urlencoding::encode(scope)
                ))
            }
            _ => Err(anyhow::anyhow!("Unknown provider: {}", self.provider)),
        }
    }

    /// Parse the redirect URL and extract the authorization code
    pub fn parse_redirect_url(&self, redirect_url: &str) -> Result<(String, String)> {
        let url = url::Url::parse(redirect_url)?;

        let mut code = None;
        let mut state = None;

        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "code" => code = Some(value.to_string()),
                "state" => state = Some(value.to_string()),
                _ => {}
            }
        }

        let code =
            code.ok_or_else(|| anyhow::anyhow!("No 'code' parameter found in redirect URL"))?;
        let state = state.unwrap_or_default();

        Ok((code, state))
    }

    /// Exchange authorization code for access token
    pub async fn exchange_code(&self, code: &str) -> Result<ProviderToken> {
        match self.provider.as_str() {
            "antigravity" => {
                // Exchange code with Google OAuth token endpoint
                let client_id =
                    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
                let client_secret = "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";
                let redirect_uri = "http://localhost:51121/oauth-callback";

                let client = reqwest::Client::new();
                let mut params = vec![
                    ("code", code.to_string()),
                    ("client_id", client_id.to_string()),
                    ("client_secret", client_secret.to_string()),
                    ("redirect_uri", redirect_uri.to_string()),
                    ("grant_type", "authorization_code".to_string()),
                ];

                // Add PKCE verifier if present
                if let Some(verifier) = &self.code_verifier {
                    params.push(("code_verifier", verifier.to_string()));
                }

                let response = client
                    .post("https://oauth2.googleapis.com/token")
                    .form(&params)
                    .send()
                    .await?;

                if !response.status().is_success() {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    return Err(anyhow::anyhow!(
                        "Token exchange failed: {} - {}",
                        status,
                        text
                    ));
                }

                let token_data: serde_json::Value = response.json().await?;

                Ok(ProviderToken {
                    access_token: token_data["access_token"]
                        .as_str()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "No access_token in response. Raw response: {:?}",
                                token_data
                            )
                        })?
                        .to_string(),
                    refresh_token: token_data["refresh_token"].as_str().map(|s| s.to_string()),
                    expires_at: token_data["expires_in"]
                        .as_i64()
                        .map(|exp| chrono::Utc::now().timestamp() + exp),
                })
            }
            "openai" => Err(anyhow::anyhow!(
                "OpenAI OAuth requires official client credentials from OpenAI. Contact OpenAI for partnership."
            )),
            _ => Err(anyhow::anyhow!("Unknown provider: {}", self.provider)),
        }
    }

    pub async fn refresh_access_token(&self, refresh_token: &str) -> Result<ProviderToken> {
        match self.provider.as_str() {
            "antigravity" => {
                let client_id =
                    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
                let client_secret = "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";

                let client = reqwest::Client::new();
                let response = client
                    .post("https://oauth2.googleapis.com/token")
                    .form(&[
                        ("refresh_token", refresh_token),
                        ("client_id", client_id),
                        ("client_secret", client_secret),
                        ("grant_type", "refresh_token"),
                    ])
                    .send()
                    .await?;

                let token_data: serde_json::Value = response.json().await?;

                Ok(ProviderToken {
                    access_token: token_data["access_token"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("No access_token in response"))?
                        .to_string(),
                    refresh_token: token_data["refresh_token"]
                        .as_str()
                        .map(|s| s.to_string())
                        .or_else(|| Some(refresh_token.to_string())),
                    expires_at: token_data["expires_in"]
                        .as_i64()
                        .map(|exp| chrono::Utc::now().timestamp() + exp),
                })
            }
            "openai" => Err(anyhow::anyhow!(
                "OpenAI OAuth requires official client credentials from OpenAI. Contact OpenAI for partnership."
            )),
            _ => Err(anyhow::anyhow!("Unknown provider: {}", self.provider)),
        }
    }

    /// Complete the full OAuth flow
    pub async fn complete_flow(&mut self, redirect_url: &str) -> Result<()> {
        let (code, state) = self.parse_redirect_url(redirect_url)?;

        // Check state if we have a verifier
        if let Some(verifier) = &self.code_verifier
            && state != *verifier
        {
            return Err(anyhow::anyhow!(
                "State mismatch! Possible CSRF attack. Expected: {}, Got: {}",
                verifier,
                state
            ));
        }

        let token = self.exchange_code(&code).await?;

        let mut tokens = OAuthTokens::load()?;
        tokens.set(self.provider.clone(), token);
        tokens.save()?;

        println!("✓ Successfully authenticated with {}", self.provider);
        Ok(())
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(&mut s, "{:02x}", b).expect("Writing to string should not fail");
    }
    s
}
