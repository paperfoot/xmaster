# Codex Audit Findings (GPT-5.4, 262K tokens)

## CRITICAL
1. **Timestamp comparison broken** (tracker.rs, store.rs, bookmarks.rs): RFC3339 vs SQLite datetime() strings compared lexicographically — WRONG results. Fix: use Unix seconds.

## HIGH
2. **Scheduled media posts broken** (scheduler.rs): Stores file paths but fire() passes them as media_ids. Fix: upload during fire().
3. **Schedule fire race condition** (scheduler.rs): Two concurrent runners can post same item. Fix: claim rows atomically (pending→sending) before posting.
4. **Bookmark auth inconsistent** (bookmarks_cmd.rs): bookmarks list/bookmark/unbookmark still use OAuth1 via XApi, but bookmarks require OAuth2. Fix: move all bookmark ops to OAuth2.
5. **Env var auth broken** (config.rs): XMASTER_API_KEY doesn't map to keys.api_key because of .split("_"). Fix: use explicit env mappings or __ separator.
6. **config set writes 0644 permissions** (config_cmd.rs): Exposes API secrets. Fix: set 0600 after write.
7. **Thread exits 0 on failure** (thread.rs): Returns success even when no tweets posted. Fix: return error if failed > 0.

## MEDIUM
8. **Bookmarks export marks read before write succeeds** (bookmarks_cmd.rs): If export to file fails, bookmarks already marked read. Fix: mark read only after successful write.
9. **Bookmarks export --json corrupts stdout** (bookmarks_cmd.rs): Raw markdown mixed with JSON envelope. Fix: markdown as JSON field.
10. **track run fails on clean install** (tracker.rs): timing_stats table not created. Fix: create in open().
11. **timeline without --user shows own posts, not home feed** (timeline.rs): Wrong endpoint.

## LOW
12. **parse_tweet_id fails on /photo/1 URLs** (cli.rs): Returns "1" not tweet ID. Fix: extract segment after "status".
13. **--quiet flag is dead** (cli.rs): Wired but never read.
