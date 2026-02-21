# Auth Surface Map

This map captures public ingress endpoints in `crates/nanobot-core/src/gateway/mod.rs` and their current auth model.

- `GET /health` -> no auth (public health probe)
- `GET /metrics` -> no auth (operational metrics endpoint)
- `GET /api/bootstrap` -> optional bearer (`Authorization`) for enriched settings; falls back to redacted response when unauthenticated
- `GET/PATCH /api/channels/config` -> requires admin auth (`is_settings_authorized`)
- `POST /api/channels/:id/verify` -> requires admin auth (`is_settings_authorized`)
- `GET/PATCH /api/settings` -> requires admin auth (`is_settings_authorized`)
- `POST /api/settings/auth/google/connect` -> requires admin auth (`is_settings_authorized`)
- `POST /api/settings/auth/google/complete` -> requires admin auth (`is_settings_authorized`)
- `POST /api/settings/security/profile` -> requires admin auth (`is_settings_authorized`)
- `GET /api/settings/doctor` -> requires admin auth (`is_settings_authorized`)
- `GET /api/skills` -> requires admin auth (`is_settings_authorized`)
- `POST /api/skills/install` -> requires admin auth (`is_settings_authorized`)
- `GET /api/skills/:id/schema` -> no auth (schema discovery)
- `POST /api/skills/:id/config` -> requires admin auth (`is_settings_authorized`)
- `POST /api/skills/:id/test` -> no auth (runtime probe)
- `GET /ws` -> session bootstraps with server-issued token; per-message token required by default
- `POST /webhooks/teams` -> webhook token and/or HMAC signature auth, timestamp window + nonce replay defense
- `POST /webhooks/google_chat` -> webhook token and/or HMAC signature auth, timestamp window + nonce replay defense

Notes:
- `NANOBOT_ENV=production` enforces secure WS posture at startup (`NANOBOT_GATEWAY_REQUIRE_TOKEN=true`, no insecure override).
- Webhook signed mode supports key rotation via `<ENV>_ACTIVE` and `<ENV>_PREVIOUS`.
