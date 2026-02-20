# Codex Proxy Research Findings (2026-02-09)

## Core Discovery: All LLM Tools Are Stateless on HTTP

Every major coding tool sends the **full conversation history** with each API request. None use server-side stateful sessions on their HTTP paths:

| Tool | API Format | Stateless? | Base URL Config |
|------|-----------|-----------|----------------|
| Claude Code | Anthropic Messages API | Yes | `ANTHROPIC_BASE_URL` |
| Codex CLI | OpenAI Responses API | **Yes on HTTP** (no `previous_response_id`) | `OPENAI_BASE_URL` |
| OpenCode | Vercel AI SDK (native per provider) | Yes (with summarization) | `baseURL` in `opencode.json` |
| KiloCode | Anthropic-canonical, adapted per provider | Yes | `baseURL` in VS Code settings |
| Aider | LiteLLM (Chat Completions / Anthropic Messages) | Yes (with history summarization) | `OPENAI_API_BASE` env var |
| Continue.dev | Native per provider | Yes | `apiBase` per model in config |
| Gemini CLI | Gemini `generateContent` (native) | Yes | `GOOGLE_GEMINI_BASE_URL` env var |

**Implication**: Aperture's proxy has full context control (edit, compress, reorder, remove blocks) for ALL of these tools.

## Codex CLI Internals

### HTTP Path (SSE Streaming) — STATELESS
- Request struct `ResponsesApiRequest` has **no `previous_response_id` field**
- Sends full `input: &[ResponseItem]` array every request
- Response `response_id` is explicitly discarded: `response_id: _` in pattern match
- `ContextManager.items: Vec<ResponseItem>` accumulates across turns, sent in full each time

### WebSocket Path — Optional Optimization Only
- WebSocket v2 CAN use `previous_response_id` as optimization for incremental appends
- Falls back gracefully to full input if ID changes or connection resets
- `websocket_last_response_id` cleared on reconnect/new turn
- Not a hard dependency — just a bandwidth optimization

### App-Server
- Respects `OPENAI_BASE_URL` env var (reads in `create_openai_provider()`)
- ChatGPT auth mode default upstream: `chatgpt.com/backend-api/codex`
- API key auth mode default upstream: `api.openai.com/v1`
- `OPENAI_BASE_URL` overrides both

### Auth Modes
- **API key** (`sk-*` prefix): Routes to `api.openai.com/v1`
- **ChatGPT subscription** (non-`sk-` Bearer): Routes to `chatgpt.com/backend-api/codex`
- ChatGPT backend expects **bare paths** (`/responses`), NOT `/v1/responses`

## Current Approach: Token-Based Upstream Routing

### What's implemented
1. Proxy detects Bearer token prefix: `sk-*` → OpenAI API, else → ChatGPT backend
2. Path normalization skips `/v1/` prefix for ChatGPT upstream
3. `UpstreamConfig` has three URLs: `anthropic_url`, `openai_url`, `chatgpt_codex_url`

### Current blocker
ChatGPT subscription routing gets "stream disconnected before completion". The 401 is fixed (correct upstream) but connection drops during streaming.

### Likely causes to investigate
1. **Missing headers/cookies**: ChatGPT backend may require headers beyond Bearer token (session cookies, CSRF tokens, user-agent strings, Codex-specific headers like `x-codex-turn-state`)
2. **WebSocket vs SSE**: Codex might prefer WebSocket when talking to ChatGPT backend — our proxy only handles HTTP/SSE
3. **TLS/SNI issues**: Proxy → ChatGPT through reqwest might have TLS negotiation differences
4. **Client fingerprinting**: ChatGPT backend might reject non-Codex user agents
5. **Request body format**: ChatGPT backend might expect slightly different JSON schema than standard Responses API

### Debug steps for next session
1. Run Codex directly (no proxy) with `RUST_LOG=debug` and capture the exact request headers/body
2. Run through proxy and compare — diff the headers
3. Test with `curl -v` to `chatgpt.com/backend-api/codex/responses` with the same Bearer token
4. Check if proxy is stripping/modifying headers that ChatGPT backend requires
5. Check if Codex app-server sends additional auth headers (cookies, session tokens)

## Alternative Approaches If Current Method Fails

### Option A: Response ID Remapping (Fork on Edit)
Instead of always proxying, only intervene when edits happen:
1. Normal flow: forward requests transparently, track response IDs
2. When user edits a block: intercept next request, strip `previous_response_id`, reconstruct full conversation from engine blocks with edits applied, send as fresh conversation
3. Return new `response_id` to Codex — it chains from edited version going forward
4. One "expensive" call per edit, then back to normal caching

**Pros**: Minimal overhead when no edits, preserves server-side caching
**Cons**: More complex state management, only useful if ChatGPT backend works at all

### Option B: Force API Key Mode
If user has both Codex subscription AND API key access (Codex Pro includes API):
1. Set `OPENAI_BASE_URL` AND override the auth mode
2. Codex uses API key auth → routes to `api.openai.com` → no ChatGPT backend issues
3. User might need to configure API key in Codex config

**Pros**: Simple, uses well-tested API path
**Cons**: Requires user to have/configure API key, might use different billing

### Option C: Intercept at App-Server Level
Instead of proxying HTTP, intercept the Codex app-server's own communication:
1. Spawn Codex app-server with modified config pointing to our proxy
2. Or: inject `OPENAI_BASE_URL` specifically for the app-server process
3. This gives deeper control than terminal-level env vars

**Pros**: Deeper interception
**Cons**: Complex process management, app-server config might differ from CLI config

### Option D: Hybrid Bridge + Proxy
Use the bridge for observation (read local history) + proxy for outbound modification:
1. Bridge reads `~/.codex/sessions/` for block visualization
2. Proxy intercepts outbound and applies hot patches
3. Best of both worlds — even if streaming breaks, observation still works

**Pros**: Graceful degradation, observation always works
**Cons**: Two capture paths to maintain

### Option E: Use OpenCode with Codex Subscription Instead
If Codex CLI proxy is too problematic:
1. OpenCode uses Vercel AI SDK (standard Chat Completions, fully stateless)
2. Sam Altman confirmed Codex subscription works with OpenCode
3. `baseURL` trivially configurable in `opencode.json`
4. Full proxy control, no ChatGPT backend complications

**Pros**: Works today, fully proven proxy path
**Cons**: Different tool UX from Codex CLI

## Responses API Features for Later Phases

### Conversation Branching (Phase 5: Memory Lifecycle)
- `previous_response_id` can point to ANY response in a chain, not just the latest
- Creates a fork — model sees only history up to the fork point
- Maps directly to Aperture's snapshot branching system
- When user creates a snapshot and forks, could create a server-side branch

### Conversations API: Item-Level CRUD (Phase 3+)
OpenAI has a separate Conversations API with granular control:
- `POST /v1/conversations` — Create conversation (up to 20 initial items)
- `GET /v1/conversations/{id}/items` — List items (paginated)
- `POST /v1/conversations/{id}/items` — Add items (up to 20 at a time)
- `DELETE /v1/conversations/{id}/items/{item_id}` — Delete individual messages
- Conversations persist indefinitely (no 30-day TTL)
- `conversation` param and `previous_response_id` are mutually exclusive

**Use for context manipulation**: Instead of hot-patching request bodies, could use Conversations API to pre-edit the server-side state, then reference the conversation in the next request. More authoritative than hot patches.

### Response Deletion (Cleanup)
- `DELETE /v1/responses/{response_id}` — Delete a specific stored response
- Non-cascading (doesn't delete children in chain)
- Could use for cleanup after forking or when user removes blocks

### `store: false` (Ephemeral Mode)
- Setting `store: false` makes responses ephemeral — not persisted, can't be chained
- Useful for one-off requests where you don't want server-side state
- But breaks `previous_response_id` chaining — so can't use it mid-conversation
- Could use for "preview" mode — send a test request without polluting the chain

### Token Billing Note
All previous input tokens in a `previous_response_id` chain are **re-billed as input tokens** on each call. Server-side state is a convenience feature, not a cost optimization. The full context window is reconstructed server-side every time.

## Provider Capability Matrix

| Capability | Anthropic | OpenAI Chat | OpenAI Responses | Codex ChatGPT | Gemini |
|---|---|---|---|---|---|
| **Proxy intercept** | ✅ Full | ✅ Full | ✅ Full | ⚠️ WIP | ✅ Full (needs parser) |
| **Hot patch** | ✅ `messages[]` | ✅ `messages[]` | ✅ `input[]` | ⚠️ WIP | ❌ Not yet |
| **Block extraction** | ✅ | ✅ | ✅ | ⚠️ WIP | ❌ Not yet |
| **Base URL config** | `ANTHROPIC_BASE_URL` | `OPENAI_API_BASE` | `OPENAI_BASE_URL` | `OPENAI_BASE_URL` | `GOOGLE_GEMINI_BASE_URL` |
| **Wire format** | Anthropic Messages | Chat Completions | Responses API | Responses API | `generateContent` |
| **Branching** | N/A (stateless) | N/A (stateless) | ✅ `previous_response_id` | ✅ Same | ❌ Unknown |
| **Server-side CRUD** | ❌ | ❌ | ✅ Conversations API | ✅ Same | ❌ |
