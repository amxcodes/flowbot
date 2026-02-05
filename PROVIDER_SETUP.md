# Provider Configuration Guide

## Overview

Nanobot-rs supports three providers:
1. **OpenRouter** - API key (simple)
2. **Antigravity (Google AI Studio)** - API key OR OAuth
3. **OpenAI ChatGPT Plus/Pro** - OAuth (subscription access)

---

## 1. OpenRouter (Easiest - Already Working)

### Setup
1. Get API key from [openrouter.ai](https://openrouter.ai)
2. Edit `config.toml`:
```toml
[providers.openrouter]
api_key = "sk-or-v1-YOUR_KEY_HERE"
```

### Available Models
- `anthropic/claude-3-opus`
- `anthropic/claude-3-5-sonnet`
- `openai/gpt-4-turbo`
- `google/gemini-pro`
- Many more at https://openrouter.ai/models

### Usage
```powershell
.\target\release\nanobot-rs.exe chat --provider openrouter
```

---

## 2. Antigravity (Google AI Studio / Gemini)

### Important: Antigravity Platform vs Gemini API

**Antigravity** refers to Google's AI-powered development platform (released Nov 2025). However, to access Gemini models, you use **Google AI Studio** API keys.

### Option A: API Key (Recommended for Testing)

1. Go to [Google AI Studio](https://makersuite.google.com/app/apikey)
2. Create new API key
3. Add to config:

```toml
[providers.antigravity]
api_key = "AI..." # Google AI Studio API key
```

### Option B: OAuth (Production)

For stricter access control and user-specific data:

1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Create OAuth 2.0 Client ID
3. Set redirect URI: `http://localhost:8080/callback`
4. Run:
```powershell
.\target\release\nanobot-rs.exe login antigravity
```

### Available Models (Google AI Studio)

**Latest Gemini Models (2025):**
- `gemini-3-pro` - Most powerful, released Nov 2025
- `gemini-3-flash` - Fast, efficient
- `gemini-2.5-pro` - Previous generation
- `gemini-2.5-flash` - Previous fast model

**Model Access:**
- API Key: Use model names like `gemini-3-pro`
- OAuth: Same, but with user-specific access control

### Base URL
```
https://generativelanguage.googleapis.com/v1beta
```

### Environment Variables for Testing
```bash
GEMINI_API_KEY=AI...YOUR_KEY_HERE
```

---

## 3. OpenAI ChatGPT Plus/Pro (Subscription Access)

### Important: Subscription vs API

**ChatGPT Plus/Pro subscription ($20/month) is SEPARATE from OpenAI API.**

- ❌ ChatGPT Plus does NOT include API access
- ❌ API keys do NOT work with Plus subscription
- ✅ OAuth allows using Plus subscription in apps like OpenCode

### How OpenCode Does It

OpenCode uses an **OAuth plugin** (`opencode-openai-codex-auth`) that:
1. Opens browser for OpenAI login
2. User authorizes with Plus/Pro account
3. Gets OAuth access token
4. Uses subscription models instead of API billing

### OAuth Flow (Like OpenCode)

**Step 1: Get OAuth Client Credentials**

Unfortunately, OpenAI's OAuth for ChatGPT subscriptions is currently:
- Only available through official partnerships
- Not publicly available for third-party apps
- Requires special OAuth client ID from OpenAI

**Current Status (Feb 2026):**
- OpenCode has official partnership
- Individual developers cannot get OAuth client ID
- OpenAI restricts this to prevent abuse

### Alternative: OpenAI API

If you have OpenAI API credits:

```toml
[providers.openai]
api_key = "sk-proj-..." # OpenAI API key
```

Models available:
- `gpt-5` (released Aug 2025)
- `gpt-4.5`
- `gpt-4-turbo`
- `gpt-4o`

Base URL: `https://api.openai.com/v1`

---

## Implementation Details

### Current nanobot-rs Code Status

**✅ Implemented:**
- Config system
- OAuth flow structure
- Provider switching

**⚠️ Needs Real OAuth Credentials:**

For ChatGPT Plus to work like OpenCode, you would need:

1. **Official OpenAI OAuth Client ID** (not publicly available)
2. Update `oauth.rs` with real client ID:

```rust
"openai" => {
    let client_id = "OFFICIAL_OPENAI_CLIENT_ID"; // Need from OpenAI
    let redirect_uri = "http://localhost:8080/callback";
    let scope = "openid email";
    
    Ok(format!(
        "https://auth.openai.com/authorize?client_id={}&redirect_uri={}&response_type=code&scope={}",
        client_id, redirect_uri, scope
    ))
}
```

3. Token exchange endpoint: `https://auth.openai.com/oauth/token`

---

## Recommended Setup (Today)

### For Development & Testing:

**Option 1: OpenRouter (Easiest)**
```toml
default_provider = "openrouter"
[providers.openrouter]
api_key = "sk-or-v1-..." # Access ALL models
```

**Option 2: Google AI Studio**
```toml
default_provider = "antigravity"
[providers.antigravity]
api_key = "AI..." # Free Gemini access
```

### Model Selection in Code

Update `src/main.rs` to use appropriate model per provider:

```rust
let model_name = match provider_name {
    "openrouter" => "anthropic/claude-3-opus",
    "antigravity" => "gemini-3-pro",
    "openai" => "gpt-5",
    _ => "default-model",
};

let agent = client
    .agent(model_name)
    .preamble("You are Nanobot...")
    .build();
```

---

## Summary

| Provider | Auth Method | Cost | Models | Status |
|----------|-------------|------|--------|--------|
| OpenRouter | API Key | Pay-per-use | All providers | ✅ Working |
| Antigravity | API Key/OAuth | Free tier | Gemini 3 Pro, Flash | ✅ Working |
| OpenAI API | API Key | Pay-per-use | GPT-5, GPT-4o | ✅ Working |
| ChatGPT Plus | OAuth | $20/month | Subscription models | ❌ Need OAuth client |

**Best Path Forward:**
1. Use **OpenRouter** OR **Google AI Studio** API key
2. Both work immediately
3. ChatGPT Plus OAuth requires official partnership with OpenAI
