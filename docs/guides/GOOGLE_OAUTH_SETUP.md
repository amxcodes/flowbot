# Setting Up Google OAuth for Antigravity (Like OpenCode)

## Overview

To use Antigravity login (Google account) like OpenCode does, you need to set up Google OAuth credentials.

## Step-by-Step Setup

### 1. Create Google Cloud Project

1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Create new project (or select existing)
3. Name it: "Nanobot" or similar

### 2. Enable APIs

1. Go to **APIs & Services** → **Library**
2. Search and enable:
   - **Generative Language API**
   - **Google OAuth 2.0 API**

### 3. Create OAuth 2.0 Credentials

1. Go to **APIs & Services** → **Credentials**
2. Click **+ CREATE CREDENTIALS** → **OAuth client ID**
3. Configure OAuth consent screen (if first time):
   - User Type: **External**
   - App name: "Nanobot"
   - User support email: your email
   - Developer contact: your email
   - Scopes: Add these:
     - `openid`
     - `email`
     - `https://www.googleapis.com/auth/generative-language.tuning`
     - `https://www.googleapis.com/auth/generative-language.retriever`

4. Create OAuth Client ID:
   - Application type: **Desktop app** (or Web application)
   - Name: "Nanobot Desktop"
   - Authorized redirect URIs: `http://localhost:8080/callback`

5. Download JSON credentials file

### 4. Extract Credentials

From the downloaded JSON, you need:
```json
{
  "client_id": "123456-abcdef.apps.googleusercontent.com",
  "client_secret": "GOCSPX-..."
}
```

### 5. Update nanobot-rs Code

Edit `src/oauth.rs`:

**Line 17** - Replace:
```rust
let client_id = "YOUR_GOOGLE_CLIENT_ID.apps.googleusercontent.com";
```
With:
```rust
let client_id = "123456-abcdef.apps.googleusercontent.com"; // Your actual client ID
```

**Line 72** - Replace:
```rust
let client_id = "YOUR_GOOGLE_CLIENT_ID.apps.googleusercontent.com";
let client_secret = "YOUR_GOOGLE_CLIENT_SECRET";
```
With your actual credentials:
```rust
let client_id = "123456-abcdef.apps.googleusercontent.com";
let client_secret = "GOCSPX-your_actual_secret";
```

### 6. Rebuild

```powershell
cargo build --release
```

### 7. Test OAuth Login

```powershell
.\target\release\nanobot-rs.exe login antigravity
```

**Flow:**
1. App will print Google OAuth URL
2. Copy and open in browser
3. Log in with your Google account
4. Grant permissions
5. Browser redirects to `http://localhost:8080/callback?code=...`
6. Copy the full redirect URL
7. Paste back into the app
8. Token is saved to `~/.nanobot/tokens.json`

### 8. Use Gemini Models

Now you can use your authenticated Google account:

```powershell
.\target\release\nanobot-rs.exe chat --provider antigravity
```

This will use:
- **Model**: `gemini-3-pro` (latest)
- **Auth**: Your Google account OAuth token
- **Access**: Your Gemini API quota

## Models Available

With Google OAuth authentication, you get access to:
- `gemini-3-pro` - Most powerful (Nov 2025 release)
- `gemini-3-flash` - Fast and efficient
- `gemini-2.5-pro` - Previous generation
- `gemini-2.5-flash` - Previous fast model

## Free Tier

Google provides generous free tier for Gemini API:
- 1,500 requests per day
- No credit card required
- Access to all models

## Troubleshooting

**Error: "OAuth client not set up for this app"**
- Make sure you added all required scopes in Google Cloud Console

**Error: "Redirect URI mismatch"**
- Ensure you added `http://localhost:8080/callback` to authorized redirect URIs

**Error: "Access denied"**
- You need to click "Allow" when granting permissions in browser

## Security Note

The `client_secret` is sensitive. In a production app, you would:
1. Store it in environment variables
2. Never commit it to git
3. Use a secure backend server

For personal use, hardcoding is acceptable since the OAuth flow still requires user consent in the browser.

## Comparison with API Key

| Method | Setup | Access | Quota |
|--------|-------|--------|-------|
| API Key | 1 minute | Project-level | Shared quota |
| OAuth | 5 minutes | User-specific | Personal quota |

Both work! OAuth is better for:
- User-specific model tuning
- Personal quota tracking
- Multi-user scenarios

API key is simpler for single-user, testing.
