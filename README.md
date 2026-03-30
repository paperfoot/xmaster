<p align="center">
  <h1 align="center">xmaster</h1>
</p>

<p align="center">
  <a href="https://github.com/199-biotechnologies/xmaster/stargazers"><img src="https://img.shields.io/github/stars/199-biotechnologies/xmaster?style=social" alt="GitHub stars"></a>
  <a href="https://x.com/longevityboris"><img src="https://img.shields.io/badge/follow-%40longevityboris-black?logo=x&style=social" alt="Follow on X"></a>
  <a href="https://crates.io/crates/xmaster"><img src="https://img.shields.io/crates/v/xmaster" alt="crates.io"></a>
  <a href="https://github.com/199-biotechnologies/xmaster/blob/main/LICENSE"><img src="https://img.shields.io/github/license/199-biotechnologies/xmaster" alt="License"></a>
</p>

<p align="center">
  <strong>X CLI for humans and agents. Post, search, reply, track; all from the terminal.</strong><br>
  <em>X API v2 + xAI Grok in one Rust binary. Works with Claude Code, any AI agent, or just you.</em>
</p>

<p align="center">
  <a href="#install">Install</a> &middot;
  <a href="#quick-start">Quick Start</a> &middot;
  <a href="#commands">Commands</a> &middot;
  <a href="#for-ai-agents">For AI Agents</a> &middot;
  <a href="#configuration">Configuration</a>
</p>

---

I built xmaster because I wanted my AI agents to handle X for me; find interesting posts in my niche, draft replies in my voice, track what's working, and make the whole experience less of a chore. Not for spamming or automation theatre; just a more convenient way to stay engaged when you'd rather be building things.

A single Rust binary with two search backends (X API v2 + xAI/Grok), structured JSON output for agents, and pre-flight intelligence that catches bad posts before they go out.

```bash
xmaster post "Hello from the command line"
```

---

## Why xmaster

Most X tools make you pick between the official API and scraping. xmaster gives you both; plus the bits nobody else builds.

- **Two search backends** — X API v2 for structured queries. xAI/Grok for "find me interesting posts about longevity from today." Use whichever fits.
- **Agent-friendly** — JSON output, semantic exit codes, machine-readable `agent-info` with measurement coverage (measurable/proxy/blind signals) and workflow handoffs. Auto-JSON when piped.
- **Action-centric pre-flight** — Estimates 9 proxy signals aligned with the 2026 X algorithm (reply, quote, profile_click, follow_author, share_via_dm, dwell, media_expand, negative_risk) and scores per goal (replies, quotes, shares, follows, impressions). Context-aware for replies, quotes, and media posts.
- **Engagement intelligence** — Adaptive opportunity scoring for reply targets based on reciprocity, ROI, size fit, and topicality. Reply style classification (question, data point, counterpoint, anecdote, humor) with outcome tracking.
- **Reply bypass** — X blocks programmatic replies to strangers. xmaster handles it automatically via web session fallback. You just run `reply` and it works.
- **Writing style** — Save how you write. Your agent drafts in your voice, not generic AI slop.
- **Single binary** — ~8MB Rust binary, fast startup. No Python, no Node, no Docker.

## Install

**One-liner (macOS / Linux):**
```bash
curl -fsSL https://raw.githubusercontent.com/199-biotechnologies/xmaster/master/install.sh | sh
```

**Homebrew:**
```bash
brew tap 199-biotechnologies/tap
brew install xmaster
```

**Cargo (crates.io):**
```bash
cargo install xmaster
```

**Cargo (from source):**
```bash
cargo install --git https://github.com/199-biotechnologies/xmaster
```

## Quick Start

```bash
# 1. Get your X API keys from https://developer.x.com
#    You need: API Key, API Secret, Access Token, Access Token Secret

# 2. Configure credentials
xmaster config set keys.api_key YOUR_API_KEY
xmaster config set keys.api_secret YOUR_API_SECRET
xmaster config set keys.access_token YOUR_ACCESS_TOKEN
xmaster config set keys.access_token_secret YOUR_ACCESS_TOKEN_SECRET

# 3. (Optional) Add xAI key for AI-powered search
xmaster config set keys.xai YOUR_XAI_KEY

# 4. Verify setup
xmaster config check

# 5. (Optional) Enable reply fallback — auto-captures cookies from your browser
xmaster config web-login

# 6. (Optional) Set your writing style — agents will write in your voice
xmaster config set style.voice "your style description"

# 7. (Optional) Enable X Premium features (25k char limit instead of 280)
xmaster config set account.premium true

# 8. Post
xmaster post "Hello from xmaster"
```

## Commands

### Posting & Engagement

| Command | Description | Example |
|---------|-------------|---------|
| `post` | Post (text, media, reply, quote, poll) | `xmaster post "Hello world"` |
| `reply` | Reply to a post (auto-bypasses API restrictions) | `xmaster reply 1234567890 "Great point"` |
| `thread` | Post a multi-tweet thread | `xmaster thread "First" "Second" "Third"` |
| `delete` | Delete a post | `xmaster delete 1234567890` |
| `like` | Like a tweet (ID or URL) | `xmaster like 1234567890` |
| `unlike` | Unlike a tweet | `xmaster unlike 1234567890` |
| `retweet` | Retweet a tweet | `xmaster retweet 1234567890` |
| `unretweet` | Undo a retweet | `xmaster unretweet 1234567890` |
| `bookmark` | Bookmark a tweet | `xmaster bookmark 1234567890` |
| `unbookmark` | Remove a bookmark | `xmaster unbookmark 1234567890` |

### Social Graph

| Command | Description | Example |
|---------|-------------|---------|
| `follow` | Follow a user | `xmaster follow elonmusk` |
| `unfollow` | Unfollow a user | `xmaster unfollow elonmusk` |
| `followers` | List a user's followers | `xmaster followers elonmusk -c 50` |
| `following` | List who a user follows | `xmaster following elonmusk -c 50` |

### Reading Posts

| Command | Description | Example |
|---------|-------------|---------|
| `read` | Full post lookup (text, author, date, metrics, media) | `xmaster read 1234567890` |
| `replies` | Get all replies/comments on a post | `xmaster replies 1234567890 -c 30` |
| `metrics` | Detailed engagement metrics | `xmaster metrics 1234567890` |
| `timeline` | View home or user timeline | `xmaster timeline --user elonmusk --since 24h --sort impressions` |
| `mentions` | View your mentions | `xmaster mentions -c 20` |
| `user` | Get user profile info | `xmaster user elonmusk` |
| `me` | Get your own profile info | `xmaster me` |
| `followers` | List your followers | `xmaster followers longevityboris -c 100` |
| `following` | List who you follow | `xmaster following longevityboris` |

The `read` command returns everything about a post in one call: author, full text, date, likes, retweets, replies, impressions, bookmarks, and media URLs. Accepts tweet IDs or full x.com URLs.

### Bookmark Intelligence

| Command | Description | Example |
|---------|-------------|---------|
| `bookmarks list` | List recent bookmarks | `xmaster bookmarks list -c 20` |
| `bookmarks sync` | Archive bookmarks locally (survives tweet deletion) | `xmaster bookmarks sync -c 200` |
| `bookmarks search` | Search archived bookmarks | `xmaster bookmarks search "longevity"` |
| `bookmarks export` | Export as markdown | `xmaster bookmarks export -o bookmarks.md` |
| `bookmarks digest` | Weekly bookmark summary | `xmaster bookmarks digest -d 7` |
| `bookmarks stats` | Bookmark statistics | `xmaster bookmarks stats` |

The `sync` command archives bookmark content locally in SQLite. Even if the original tweet gets deleted, your local copy survives. Local search is instant, and the digest helps you actually read what you save. Sync regularly to keep your archive fresh.

### Engagement Intelligence

| Command | Description | Example |
|---------|-------------|---------|
| `engage recommend` | Find high-ROI reply targets in your niche | `xmaster engage recommend --topic "longevity biotech" -c 10` |
| `engage feed` | Fresh posts from big accounts to reply to now | `xmaster engage feed "AI agents" --min-followers 5000` |
| `engage watchlist add` | Track an account without following | `xmaster engage watchlist add elonmusk --topic "tech"` |
| `engage watchlist list` | List watched accounts | `xmaster engage watchlist list` |

Replies are estimated ~20x a like and DM shares ~25x (2026 algorithm estimates). `engage recommend` uses an **opportunity scorer** that ranks targets by reciprocity, reply ROI, size fit (adaptive to your follower count), topicality, and freshness. `engage feed` finds fresh posts to reply to, scored by opportunity — not just recency. Reply styles are classified (question, data point, counterpoint, anecdote, humor) and tracked for outcome analysis.

### Search

| Command | Description | Example |
|---------|-------------|---------|
| `search` | Search tweets (X API v2) | `xmaster search "rust lang" --mode recent` |
| `search-ai` | AI-powered search (xAI/Grok) | `xmaster search-ai "latest AI news"` |
| `trending` | Get trending topics (xAI) | `xmaster trending --region US` |

### Direct Messages

| Command | Description | Example |
|---------|-------------|---------|
| `dm send` | Send a DM | `xmaster dm send alice "Hey!"` |
| `dm inbox` | View DM inbox | `xmaster dm inbox -c 20` |
| `dm thread` | View a DM conversation | `xmaster dm thread CONV_ID` |

### Scheduling

| Command | Description | Example |
|---------|-------------|---------|
| `schedule add` | Schedule a post for later | `xmaster schedule add "text" --at "2026-03-24 09:00"` |
| `schedule add --at auto` | Auto-pick best posting time | `xmaster schedule add "text" --at auto` |
| `schedule list` | List scheduled posts | `xmaster schedule list --status pending` |
| `schedule cancel` | Cancel a scheduled post | `xmaster schedule cancel sched_abc123` |
| `schedule reschedule` | Change post time | `xmaster schedule reschedule sched_abc --at "2026-03-25 10:00"` |
| `schedule fire` | Execute due posts (cron) | `xmaster schedule fire` |
| `schedule setup` | Install launchd auto-scheduler | `xmaster schedule setup` |

Posts are stored locally in SQLite — no X Ads API needed, pure local scheduling. The `launchd` daemon fires every 5 minutes on macOS. Use `--at auto` to let xmaster pick the best posting time from your engagement history. Missed schedules are handled with a 5-minute grace period.

### Pre-Flight Intelligence

| Command | Description | Example |
|---------|-------------|---------|
| `analyze` | Score a post before publishing | `xmaster analyze "your text" --goal replies` |
| `suggest best-time` | Best posting time from history | `xmaster suggest best-time` |
| `suggest next-post` | Cannibalization guard | `xmaster suggest next-post` |
| `report daily` | Daily performance digest | `xmaster report daily` |
| `report weekly` | Weekly performance digest | `xmaster report weekly` |
| `track run` | Snapshot recent post metrics | `xmaster track run` |
| `track followers` | Track follower changes (new/lost) | `xmaster track followers` |
| `track growth` | Follower growth history | `xmaster track growth -d 30` |
| `engage recommend` | Find high-ROI reply targets | `xmaster engage recommend --topic "AI" -c 10` |
| `inspire` | Browse discovered posts library | `xmaster inspire --topic "longevity" --min-likes 50` |

### Discovered Posts Library

Every search, timeline view, and post read automatically caches posts into a local SQLite library — zero extra API calls. Over time, this builds a personal collection of posts from your niche.

```bash
xmaster inspire --topic "longevity" --min-likes 100  # Browse by topic
xmaster inspire --author "naval" --count 10           # Browse by author
xmaster inspire --json                                # Pipe to jq for analysis
```

### Lists

| Command | Description | Example |
|---------|-------------|---------|
| `lists create` | Create a list | `xmaster lists create "AI Builders"` |
| `lists add` | Add user to a list | `xmaster lists add LIST_ID username` |
| `lists timeline` | View a list's timeline | `xmaster lists timeline LIST_ID` |
| `lists mine` | View your lists | `xmaster lists mine` |

### Moderation

| Command | Description | Example |
|---------|-------------|---------|
| `block` | Block a user | `xmaster block spammer123` |
| `unblock` | Unblock a user | `xmaster unblock username` |
| `mute` | Mute a user | `xmaster mute username` |
| `unmute` | Unmute a user | `xmaster unmute username` |
| `hide-reply` | Hide a reply to your post | `xmaster hide-reply 1234567890` |
| `unhide-reply` | Unhide a reply | `xmaster unhide-reply 1234567890` |

### System

| Command | Description | Example |
|---------|-------------|---------|
| `config show` | Show config (keys masked) | `xmaster config show` |
| `config get` | Get a single config value | `xmaster config get style.voice` |
| `config set` | Set a config value | `xmaster config set keys.api_key KEY` |
| `config check` | Validate credentials | `xmaster config check` |
| `config web-login` | Auto-capture browser cookies (reply fallback) | `xmaster config web-login` |
| `config auth` | OAuth 2.0 PKCE (for bookmarks) | `xmaster config auth` |
| `agent-info` | Machine-readable capabilities + algorithm weights | `xmaster agent-info` |
| `rate-limits` | Check API quota status | `xmaster rate-limits` |
| `update` | Self-update from GitHub releases | `xmaster update` |
| `star` | Open GitHub repo to star it | `xmaster star` |

### Global Flags

| Flag | Description |
|------|-------------|
| `--json` | Force JSON output (auto-enabled when piped) |

### Post Options

```bash
# Reply to a tweet
xmaster post "Great point!" --reply-to 1234567890

# Quote tweet
xmaster post "This is important" --quote 1234567890

# Attach media (up to 4 files)
xmaster post "Check this out" --media photo.jpg --media chart.png

# Create a poll (24h default)
xmaster post "Best language?" --poll "Rust,Go,Python,TypeScript"

# Poll with custom duration (minutes)
xmaster post "Best language?" --poll "Rust,Go" --poll-duration 60

# Tweet ID or URL both work for engagement commands
xmaster like https://x.com/user/status/1234567890
```

### Search Options

```bash
# X API v2 search with mode
xmaster search "query" --mode recent      # Recent tweets (default)
xmaster search "query" --mode popular     # Popular tweets
xmaster search "query" -c 25             # Get 25 results

# AI-powered search with date filters
xmaster search-ai "CRISPR breakthroughs" --from-date 2026-01-01 --to-date 2026-03-01
xmaster search-ai "AI news" -c 20

# Trending topics
xmaster trending --region US --category technology
```

## For AI Agents

xmaster is built for AI agents from day one. Every command supports `--json` and structured error codes.

### JSON Output

```bash
# Force JSON output
xmaster --json post "Hello from my agent"

# Auto-JSON when piped
xmaster post "Hello" | jq '.data.id'
```

**Success envelope:**
```json
{
  "version": "1",
  "status": "success",
  "data": { ... },
  "metadata": {
    "elapsed_ms": 342,
    "provider": "x_api"
  }
}
```

**Error envelope:**
```json
{
  "status": "error",
  "error": {
    "code": "auth_missing",
    "message": "Authentication missing: X API credentials not configured",
    "suggestion": "Set X API credentials via env vars (XMASTER_API_KEY, etc.) or run: xmaster config set keys.api_key <key>"
  }
}
```

### Exit Codes

| Code | Meaning | Agent Action |
|------|---------|--------------|
| 0 | Success | Process results |
| 1 | Runtime error | Retry might help |
| 2 | Config error | Fix configuration |
| 3 | Auth missing | Set API key |
| 4 | Rate limited | Back off and retry |

### Agent Discovery

```bash
# Machine-readable capabilities, algorithm weights, and measurement coverage
xmaster agent-info --json
```

Returns: 64 commands, 18 capabilities, 15 algorithm signal weights, measurement coverage (7 measurable, 6 proxy-only, 9 blind), 5 workflow handoffs, and writing style config.

### Pre-flight Analysis

```bash
# Proxy-signal estimation aligned with 2026 X algorithm
xmaster analyze "your tweet text" --goal replies --json
```

Returns per-signal proxy scores (reply, quote, profile_click, follow_author, share_via_dm, dwell, media_expand, negative_risk) and per-goal scores (replies, quotes, shares, follows, impressions 0-100), plus lint issues and suggestions.

### Integration Example (Claude Code Skill)

```bash
# In a Claude Code skill, xmaster works seamlessly:
RESULT=$(xmaster --json search "topic of interest" -c 5)
echo "$RESULT" | jq '.data[] | {text: .text, author: .author}'

# Or for posting:
xmaster --json post "Automated insight" --reply-to "$TWEET_ID"
```

## Configuration

Config file lives at `~/.config/xmaster/config.toml` on all platforms.

Override with `XMASTER_CONFIG_DIR` env var.

```bash
xmaster config show       # View current config (keys masked)
xmaster config get K      # Read a single value (e.g. style.voice)
xmaster config check      # Validate all credentials
xmaster config set K V    # Set a value
```

### Environment Variables

Environment variables override the config file. Prefix `XMASTER_` with double underscore `__` for nesting:

```bash
export XMASTER_KEYS__API_KEY=your-api-key
export XMASTER_KEYS__API_SECRET=your-api-secret
export XMASTER_KEYS__ACCESS_TOKEN=your-access-token
export XMASTER_KEYS__ACCESS_TOKEN_SECRET=your-access-token-secret
export XMASTER_KEYS__XAI=your-xai-key
export XMASTER_SETTINGS__TIMEOUT=30
```

## Architecture

```
┌─────────────────────────────────────────────┐
│                 CLI Layer                    │
│   clap + comfy-table (--json / human)       │
├─────────────────────────────────────────────┤
│          Command Router + Pre-flight        │
│   Analyze, score, cannibalization guard     │
├──────────┬──────────────┬───────────────────┤
│ X API v2 │  xAI / Grok  │  Web GraphQL      │
│(OAuth1.0a│(Bearer token)│  (Cookie auth +   │
│ Post,Like│ AI search,   │   transaction ID) │
│ RT, DM,  │ Trending,    │  Reply fallback   │
│ Search,  │ Semantic     │  when API blocks  │
│ Timeline)│ search       │  replies          │
├──────────┴──────────────┴───────────────────┤
│  Rate Limiter │ Intel Store │ Scheduler     │
│  (header-based)│  (SQLite)  │  (launchd)    │
├─────────────────────────────────────────────┤
│             Config (figment)                │
│   TOML + env vars + browser cookies         │
└─────────────────────────────────────────────┘
```

### Key Design Decisions

- **OAuth 1.0a signing** — X API v2 authentication via `reqwest-oauth1`.
- **Dual search** — `search` uses X API v2 (structured, filterable). `search-ai` uses xAI/Grok (semantic, AI-powered).
- **Rate-limit aware** — Parses `x-rate-limit-*` headers and backs off automatically when hitting API quotas.
- **Auto-JSON detection** — Output is JSON when piped, human-readable tables when in a terminal.
- **URL or ID** — Engagement commands accept both tweet URLs and raw IDs.
- **Media uploads** — Chunked upload flow with base64 encoding for images and video.

## Rate Limits

xmaster respects X API v2 rate limits by parsing response headers and backing off when needed:

| Endpoint | Rate Limit |
|----------|-----------|
| POST /tweets | 200 / 15 min (user) |
| GET /tweets/search | 450 / 15 min (app) |
| POST /likes | 200 / 15 min (user) |
| POST /retweets | 300 / 15 min (user) |
| GET /dm_conversations | 300 / 15 min (user) |

When rate limited, xmaster returns exit code 4 with a structured error including retry guidance.

## Updating

```bash
xmaster update             # Self-update from GitHub releases
xmaster update --check     # Check without installing
```

## Building from Source

```bash
git clone https://github.com/199-biotechnologies/xmaster
cd xmaster
cargo build --release
# Binary at target/release/xmaster
```

## Works With

- [Claude Code](https://github.com/anthropics/claude-code) — as a native skill or bash tool
- [OpenClaw](https://github.com/openclaw/openclaw) — as a skill (`skills/xmaster`)
- Any AI agent that can shell out and parse JSON

## License

MIT

---

Built by [Boris Djordjevic](https://x.com/longevityboris) at [199 Biotechnologies](https://github.com/199-biotechnologies).
Building longevity tools, pet health AI, and open source CLIs. Follow along on [X](https://x.com/longevityboris).
