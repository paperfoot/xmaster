# X Algorithm Source Code Analysis

Deep technical analysis of X's recommendation algorithm from two open-sourced repositories:
- **2023**: [twitter/the-algorithm](https://github.com/twitter/the-algorithm) (Scala/Java)
- **2026**: [xai-org/x-algorithm](https://github.com/xai-org/x-algorithm) (Rust, Grok-powered)

Focus: understanding algorithmic signals for small account growth (~100 followers, dormant account reactivation).

## Documents

| # | File | Focus |
|---|---|---|
| 01 | [Candidate Generation (2023)](01-candidate-generation-2023.md) | SimClusters, Real Graph, EarlyBird, GraphJet/UTEG, CrMixer, TwHIN — how tweets enter the ranking pipeline |
| 02 | [Ranking Pipeline (2023)](02-ranking-pipeline-2023.md) | Heavy Ranker (MaskNet), exact engagement weights, Light Ranker, Home Mixer orchestration, Premium boosts |
| 03 | [Grok Algorithm (2026)](03-grok-algorithm-2026.md) | Phoenix two-tower model, Grok transformer ranking, weighted_scorer.rs, 15 output probabilities, architectural changes |
| 04 | [Small Account Deep Dive](04-small-account-deep-dive.md) | TweepCred scoring, cold start analysis, discovery mechanisms, concrete scenario for ~100 follower accounts |
| 05 | [Growth Playbook](05-growth-playbook.md) | Actionable strategy with every recommendation citing specific code variables and engagement weights |
| 06 | [Penalties & Negative Signals](06-penalties-and-negative-signals.md) | Negative weights, visibility filtering, content/network penalties, 2026 Grok sentiment analysis, recovery |

## Key Numbers

| Signal | Weight | Multiplier vs Like |
|---|---|---|
| Reply + author engages back | +75.0 | 150x |
| Reply | +13.5 | 27x |
| Profile click + engage | +12.0 | 24x |
| Good click (stay 2+ min) | +10.0 | 20x |
| Retweet | +1.0 | 2x |
| Like | +0.5 | 1x (baseline) |
| Block/mute/show less | -74.0 | -148x |
| Report | -369.0 | -738x |

## Critical Thresholds for Small Accounts

- **TweepCred < 65**: Only 3 tweets/day enter the ranking pipeline
- **Following/follower ratio > 0.6**: Exponential PageRank penalty via `exp(5 * (ratio - 0.6))`
- **SimClusters Known-For**: Requires `minActiveFollowers = 400` — small accounts excluded
- **Tweet half-life**: 360 minutes (6 hours) with decay rate 0.003
- **Real Graph decay**: `ONE_MINUS_ALPHA = 0.955` (~7-day half-life on engagement signals)
- **Premium boost**: 4x in-network, 2x out-of-network (worthless without Real Graph edges)

## TL;DR for ~100 Follower Accounts

1. **Your estimated TweepCred is 15-35** (need 65). Only 3 tweets/day get ranked.
2. **Reply to people in your niche** — replies are 150x more valuable than likes when authors engage back.
3. **Never get reported** — one report destroys 738 likes worth of score.
4. **Fix your ratio first** — unfollow to get below 0.6 following/followers.
5. **Old dormant account > new account** — dormant follow edges can be reactivated.
6. **Don't subscribe to Premium until ~500 followers** — multipliers need non-zero edges to multiply.
7. **First 30 minutes matter** — early engagement determines algorithmic momentum.

---

*Generated 2026-03-27 by 6 parallel analysis agents reading actual source code from twitter/the-algorithm and xai-org/x-algorithm.*
