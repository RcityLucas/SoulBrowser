# Structured Schema Catalog

## Active Structured Parsers

| Schema ID | Parser Tool | Parser Module | Schema Spec | Deliver Step | Release Owner |
| --- | --- | --- | --- | --- | --- |
| `market_info_v1` | `data.parse.market_info` | `src/parsers/market_info.rs` (`parse_market_info`) | [`schemas/market_info_v1.json`](schemas/market_info_v1.json) | `data.deliver.structured` (`schema=market_info_v1`) | Structured Outputs |
| `news_brief_v1` | `data.parse.news_brief` | `src/parsers/news_brief.rs` (`parse_news_brief`) | [`schemas/news_brief_v1.json`](schemas/news_brief_v1.json) | `data.deliver.structured` (`schema=news_brief_v1`) | Structured Outputs |
| `github_repos_v1` | `data.parse.github-repo` / `github.extract-repo` | `src/agent/executor.rs::handle_parse_github_repos` | [`schemas/github_repos_v1.json`](schemas/github_repos_v1.json) | `data.deliver.structured` (`schema=github_repos_v1`) | Integrations |
| `twitter_feed_v1` | `data.parse.twitter-feed` | `src/parsers/twitter_feed.rs` (`parse_twitter_feed`) | [`schemas/twitter_feed_v1.json`](schemas/twitter_feed_v1.json) | `data.deliver.structured` (`schema=twitter_feed_v1`) | Integrations |
| `facebook_feed_v1` | `data.parse.facebook-feed` | `src/parsers/facebook_feed.rs` (`parse_facebook_feed`) | [`schemas/facebook_feed_v1.json`](schemas/facebook_feed_v1.json) | `data.deliver.structured` (`schema=facebook_feed_v1`) | Integrations |
| `hackernews_feed_v1` | `data.parse.hackernews-feed` | `src/parsers/hackernews_feed.rs` (`parse_hackernews_feed`) | [`schemas/hackernews_feed_v1.json`](schemas/hackernews_feed_v1.json) | `data.deliver.structured` (`schema=hackernews_feed_v1`) | Integrations |
| `linkedin_profile_v1` | `data.parse.linkedin-profile` | `src/parsers/linkedin_profile.rs` (`parse_linkedin_profile`) | [`schemas/linkedin_profile_v1.json`](schemas/linkedin_profile_v1.json) | `data.deliver.structured` (`schema=linkedin_profile_v1`) | Integrations |

## Current Tool Usage Summary

- Supported `data.parse.*` identifiers: `market_info`, `news_brief`, `github-repo` (alias `github.extract-repo`), `twitter-feed`, `facebook-feed`, `linkedin-profile`, and `hackernews-feed`. Planner prompts restrict custom tools to this allowlist.
- Structured deliveries go through `data.deliver.structured`; historical plans using `data.deliver.json` are normalized to the structured variant during execution.

## Upcoming Targets & Schema Drafts

### Priority ranking
1. **News portal digest (Reuters/Bloomberg/WSJ landing pages)** – broad audience, complements `news_brief_v1` with authoritative sites.
2. **Product Hunt / Reddit tech digest** – surfaces trending launches and discussions similar to HN, but with richer metadata (votes, maker handles).

### Target schema sketches

| Target | Desired Fields (initial draft) | Example Pages | Sensitivity / Rate Limits / Notes |
| --- | --- | --- | --- |
| News portal digest (`news_portal_digest_v1`) | `source`, `scrape_time`, `items[] {title, summary, url, category, published_at}`, optional `hero_image` | `https://www.reuters.com/`, `https://www.wsj.com/`, `https://www.bloomberg.com/` | Mostly public but some sites paywalled; add policy flag to skip paywalled sources. Rate limits vary; respect robots.txt guidance. |
| Product Hunt / Reddit tech digest (`launch_digest_v1`) | `source`, `items[] {rank, title, url, discussion_url, votes, author, comment_count}`, `captured_at` | `https://www.producthunt.com/`, `https://www.reddit.com/r/startups/` | Need login-aware observations for Reddit/PH filtering; consider API fallback for votes/comments if DOM obfuscated. |

## How to extend

1. Use `cargo run --bin parser_scaffold -- <name>` (see [Parser Development Guide](../guides/PARSER_DEVELOPMENT.md)).
2. Implement parsing logic + schema-specific deliver step.
3. Add/modify catalog rows plus upcoming-target notes here.
4. Update planner prompts + alias map if new tool names are introduced.
