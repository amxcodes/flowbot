# Ingress Call Graph

## Teams Webhook

1. `POST /webhooks/teams`
2. `verify_webhook_request(...)`
   - optional shared token check (`x-nanobot-webhook-token`)
   - optional HMAC body signature check (`x-nanobot-signature`)
   - timestamp skew check
   - nonce replay check (in-memory or sqlite-backed if configured)
3. JSON parse -> message normalization
4. onboarding flow (`gateway::onboarding::process_onboarding_message`)
5. pending question normalization (`tools::question`)
6. enqueue `AgentMessage` to agent channel
7. stream `StreamChunk` responses -> aggregate text
8. return response payload

## Google Chat Webhook

1. `POST /webhooks/google_chat`
2. `verify_webhook_request(...)` (same flow as Teams)
3. JSON parse -> extract user/channel/is_dm
4. onboarding flow
5. pending question normalization
6. enqueue `AgentMessage`
7. stream `StreamChunk` responses -> aggregate text
8. return response payload

## WebSocket

1. `GET /ws` upgrade
2. server generates `session_id` and signed session token
3. per-message token verification (default required)
4. request dispatch (compat RPC + send path)
5. enqueue `AgentMessage`
6. stream response chunks back over socket

## Persistence Path (Message Save)

1. message save requested from agent loop
2. bounded semaphore acquire (`NANOBOT_PERSISTENCE_BLOCKING_LIMIT`)
3. `spawn_blocking` transaction:
   - context tree mutation
   - persistence table write
   - prune check
   - daily memory markdown append
4. semaphore release

## Pairing DB Path

1. async API call in `pairing::db`
2. bounded semaphore acquire (`NANOBOT_PAIRING_DB_BLOCKING_LIMIT`)
3. `spawn_blocking` + sqlite operation
4. semaphore release
