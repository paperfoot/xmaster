# 05 -- Negative Signals and Penalties in X's 2026 Algorithm

> **Source**: [xai-org/x-algorithm](https://github.com/xai-org/x-algorithm) (January 2026 open-source release) -- Rust home-mixer + Python/JAX Phoenix model. All code references below point to `/source-2026/`.

---

## 1. Architecture Overview: Where Penalties Live

The 2026 algorithm has **two distinct layers** of negative enforcement:

| Layer | Mechanism | Effect |
|-------|-----------|--------|
| **Scoring penalties** | Predicted negative actions reduce the weighted score | Content is ranked lower, shown to fewer people |
| **Hard filters** | Binary removal -- content is dropped entirely | Content never reaches the feed at all |

Scoring is probabilistic and continuous. Filtering is absolute. Both operate simultaneously.

---

## 2. The Four Negative Signals (Weighted Scorer)

**File**: `home-mixer/scorers/weighted_scorer.rs` (lines 64-67)

The `compute_weighted_score()` function sums 19 action predictions. Four of them carry **negative weights**:

```rust
+ Self::apply(s.not_interested_score, p::NOT_INTERESTED_WEIGHT)   // negative
+ Self::apply(s.block_author_score,   p::BLOCK_AUTHOR_WEIGHT)     // negative
+ Self::apply(s.mute_author_score,    p::MUTE_AUTHOR_WEIGHT)      // negative
+ Self::apply(s.report_score,         p::REPORT_WEIGHT)            // negative
```

Each `apply()` call multiplies a predicted probability by a configured weight:

```rust
fn apply(score: Option<f64>, weight: f64) -> f64 {
    score.unwrap_or(0.0) * weight
}
```

The exact weight values live in `params.rs`, which was **excluded from the open-source release** ("Excluded from open source release for security reasons" -- `home-mixer/lib.rs`, line 5). We know the structure; we do not know the numbers. What we can deduce from the architecture is analysed in section 4.

### What each signal means

| Signal | Action Name | What it represents |
|--------|------------|-------------------|
| `not_interested_score` | `ClientTweetNotInterestedIn` | User would click "Not interested" / "Show less" |
| `block_author_score` | `ClientTweetBlockAuthor` | User would block the author after seeing this post |
| `mute_author_score` | `ClientTweetMuteAuthor` | User would mute the author after seeing this post |
| `report_score` | `ClientTweetReport` | User would report this post |

These are **predictions, not events**. The system punishes content it estimates you would dislike before you ever act on it.

---

## 3. How the Predictions Are Generated (Phoenix Scorer)

**File**: `home-mixer/scorers/phoenix_scorer.rs` (lines 129-151)

The Phoenix Scorer calls the Grok-based transformer model, which returns **log-probabilities** for each action. These are converted to probabilities via `exp()`:

```rust
let action_probs: HashMap<usize, f64> = distribution
    .top_log_probs
    .iter()
    .enumerate()
    .map(|(idx, log_prob)| (idx, (*log_prob as f64).exp()))
    .collect();
```

The model then extracts scores for all 19 actions, including the four negative ones:

```rust
PhoenixScores {
    // ... 14 positive actions ...
    not_interested_score: p.get(ActionName::ClientTweetNotInterestedIn),
    block_author_score:   p.get(ActionName::ClientTweetBlockAuthor),
    mute_author_score:    p.get(ActionName::ClientTweetMuteAuthor),
    report_score:         p.get(ActionName::ClientTweetReport),
    // ...
}
```

### What feeds the predictions

The Grok transformer ingests your **User Action Sequence** -- a chronological record of your recent engagements (likes, replies, reposts, blocks, mutes, reports, dwell times). This is fetched by `UserActionSeqQueryHydrator` and fed directly into the Phoenix model as the "history" portion of the transformer's input.

**File**: `phoenix/recsys_model.py` -- The ranking model architecture:

```
Input: [User Embedding | History Embeddings (S positions) | Candidate Embeddings (C positions)]
                                    |
                             Transformer with Candidate Isolation
                                    |
Output: [B, num_candidates, num_actions]  -- logits for each action
```

The candidate isolation attention mask (defined in `phoenix/grok.py`, `make_recsys_attn_mask()`) ensures each candidate is scored independently. A candidate cannot influence another candidate's score -- but your entire engagement history shapes every score.

The key implication: if you have historically blocked, muted, or reported content similar to a given candidate, the model learns this pattern and will predict higher P(block), P(mute), P(report) for similar content in the future. **Your negative actions train the model to pre-emptively suppress similar content.**

---

## 4. The offset_score() Function: Negative Score Compression

**File**: `home-mixer/scorers/weighted_scorer.rs` (lines 83-91)

After all 19 weighted predictions are summed into `combined_score`, the `offset_score()` function applies a critical transformation:

```rust
fn offset_score(combined_score: f64) -> f64 {
    if p::WEIGHTS_SUM == 0.0 {
        combined_score.max(0.0)
    } else if combined_score < 0.0 {
        (combined_score + p::NEGATIVE_WEIGHTS_SUM) / p::WEIGHTS_SUM * p::NEGATIVE_SCORES_OFFSET
    } else {
        combined_score + p::NEGATIVE_SCORES_OFFSET
    }
}
```

### Three constants (values hidden in params.rs)

| Constant | Meaning |
|----------|---------|
| `WEIGHTS_SUM` | Sum of all 19 absolute weight values |
| `NEGATIVE_WEIGHTS_SUM` | Sum of the absolute values of the 4 negative weights |
| `NEGATIVE_SCORES_OFFSET` | Baseline offset added to all scores |

### Mathematical analysis of the three branches

**Branch 1: `WEIGHTS_SUM == 0.0`** -- Degenerate safety case. Clamps to zero. Irrelevant in production.

**Branch 2: `combined_score < 0.0`** (the critical path for penalised content)

```
result = (combined_score + NEGATIVE_WEIGHTS_SUM) / WEIGHTS_SUM * NEGATIVE_SCORES_OFFSET
```

This is a **compression and re-mapping** of negative scores:

- `combined_score + NEGATIVE_WEIGHTS_SUM` shifts the score upward. Since `combined_score` ranges from roughly `-NEGATIVE_WEIGHTS_SUM` (maximum penalty) to `0`, this shifts the range to approximately `[0, NEGATIVE_WEIGHTS_SUM]`.
- Dividing by `WEIGHTS_SUM` normalises it to a fraction of the total weight budget: range becomes `[0, NEGATIVE_WEIGHTS_SUM / WEIGHTS_SUM]`.
- Multiplying by `NEGATIVE_SCORES_OFFSET` scales it into the offset space.

The net effect: **all negative scores are compressed into a narrow band near zero** (or near the offset baseline), rather than being allowed to go deeply negative. The worst possible score for heavily-penalised content is approximately `0` (before offset scaling), not negative infinity.

**Branch 3: `combined_score >= 0.0`** (normal content)

```
result = combined_score + NEGATIVE_SCORES_OFFSET
```

A simple additive offset. All positive scores are shifted upward by a constant.

### What this architecture reveals about asymmetry

The two branches create **fundamentally different treatment** of positive and negative scores:

1. **Positive scores scale linearly** with engagement strength. More predicted likes/replies/shares = proportionally higher score. No ceiling compression.

2. **Negative scores are compressed into a bounded band.** Even if the model predicts with near-certainty that a user would block, mute, AND report a post, the penalty is capped. The worst negative content gets a score near `0 * NEGATIVE_SCORES_OFFSET = 0`, while the best positive content can score arbitrarily high.

3. **The ratio `NEGATIVE_WEIGHTS_SUM / WEIGHTS_SUM` determines how much of the scoring budget is allocated to punishment.** If negative weights are, say, 20% of total weights, then the compressed negative band occupies at most 20% of the offset range.

4. **`NEGATIVE_SCORES_OFFSET` acts as a floor.** By adding it to positive scores and using it to scale compressed negative scores, it ensures all final scores are positive (required for the TopKScoreSelector, which treats `NEG_INFINITY` as "unscored").

### Practical implication

The architecture is biased toward **suppression over promotion**. Here is why:

- A post that generates strong positive signals (high P(like), P(reply)) gets a healthy score, but the offset means it competes from a baseline.
- A post that generates even **moderate** negative signals (e.g., P(not_interested) = 0.3, P(block) = 0.1) can easily drop its combined_score below zero, at which point it enters the compression branch and is effectively killed.
- Because negative weights are negative and positive predictions cannot cancel them directly (a post can score high on P(like) and still score high on P(not_interested) for different audience segments), **the negative predictions act as a veto**.

This is by design. The README states: "Negative actions (block, mute, report) have negative weights, pushing down content the user would likely dislike."

---

## 5. Hard Filters: Binary Removal

In addition to scoring penalties, several filters act as **absolute vetoes** -- they remove content entirely, regardless of score.

### 5.1 AuthorSocialgraphFilter

**File**: `home-mixer/filters/author_socialgraph_filter.rs`

```rust
let muted = viewer_muted_user_ids.contains(&author_id);
let blocked = viewer_blocked_user_ids.contains(&author_id);
if muted || blocked {
    removed.push(candidate);
}
```

If you have blocked or muted an author, their content is **removed before scoring even happens**. This runs in the pre-scoring filter phase (see pipeline order in `phoenix_candidate_pipeline.rs`, lines 109-120).

The block/mute lists come from `UserFeatures`:

```rust
pub struct UserFeatures {
    pub muted_keywords: Vec<String>,
    pub blocked_user_ids: Vec<i64>,
    pub muted_user_ids: Vec<i64>,
    pub followed_user_ids: Vec<i64>,
    pub subscribed_user_ids: Vec<i64>,
}
```

### 5.2 MutedKeywordFilter

**File**: `home-mixer/filters/muted_keyword_filter.rs`

Tokenises the user's muted keyword list and matches against each candidate's tweet text. Any match removes the post entirely:

```rust
if matcher.matches(&tweet_text_token_sequence) {
    removed.push(candidate);
} else {
    kept.push(candidate);
}
```

### 5.3 VFFilter (Visibility Filtering)

**File**: `home-mixer/filters/vf_filter.rs`

Runs **after selection** (post-selection phase). Uses a separate Visibility Filtering service (`xai_visibility_filtering`) with different safety levels for in-network vs. out-of-network content:

```rust
// In-network: TimelineHome safety level
// Out-of-network: TimelineHomeRecommendations safety level (stricter)
```

Content is dropped if it has a `SafetyResult` with `Action::Drop`, or any other `FilteredReason`. This catches:

- Deleted posts
- Spam
- Violence/gore
- Policy violations flagged by the safety system

Out-of-network content faces a stricter safety level (`TimelineHomeRecommendations` vs. `TimelineHome`), meaning recommended content must pass a higher bar than content from accounts you follow.

### 5.4 Other Pre-Scoring Filters

The full filter chain, in order:

| Order | Filter | Effect |
|-------|--------|--------|
| 1 | `DropDuplicatesFilter` | Remove duplicate post IDs |
| 2 | `CoreDataHydrationFilter` | Remove posts that failed metadata hydration |
| 3 | `AgeFilter` | Remove posts older than `MAX_POST_AGE` |
| 4 | `SelfTweetFilter` | Remove the viewer's own posts |
| 5 | `RetweetDeduplicationFilter` | Dedupe reposts of the same content |
| 6 | `IneligibleSubscriptionFilter` | Remove paywalled content user cannot access |
| 7 | `PreviouslySeenPostsFilter` | Remove posts already seen by the user |
| 8 | `PreviouslyServedPostsFilter` | Remove posts already served in the current session |
| 9 | `MutedKeywordFilter` | Remove posts matching muted keywords |
| 10 | `AuthorSocialgraphFilter` | Remove posts from blocked/muted authors |

These 10 filters run sequentially. A post must survive all 10 to reach the scoring stage.

---

## 6. The Scoring Pipeline: Full Penalty Chain

The scoring phase runs four scorers sequentially (`phoenix_candidate_pipeline.rs`, lines 127-132):

```
1. PhoenixScorer      -- Grok predicts P(action) for 19 actions
2. WeightedScorer     -- Combines predictions into weighted_score (with offset_score compression)
3. AuthorDiversityScorer -- Attenuates repeated authors (decay factor per appearance)
4. OONScorer          -- Multiplies out-of-network scores by OON_WEIGHT_FACTOR (<1.0)
```

A post that triggers negative predictions in step 1 gets penalised in step 2 (score compression), then may get further penalised in steps 3 and 4. The penalties stack multiplicatively.

### Author Diversity Penalty

**File**: `home-mixer/scorers/author_diversity_scorer.rs`

```rust
fn multiplier(&self, position: usize) -> f64 {
    (1.0 - self.floor) * self.decay_factor.powf(position as f64) + self.floor
}
```

The first post from an author gets a multiplier near 1.0. The second gets `decay^1`, the third `decay^2`, etc. This penalises accounts that dominate the feed -- even if their content scores well.

### Out-of-Network Penalty

**File**: `home-mixer/scorers/oon_scorer.rs`

```rust
let updated_score = c.score.map(|base_score| match c.in_network {
    Some(false) => base_score * p::OON_WEIGHT_FACTOR,
    _ => base_score,
});
```

Out-of-network content (posts from accounts the user does not follow) receives a multiplicative penalty via `OON_WEIGHT_FACTOR`. The exact value is hidden, but empirical testing suggests it is substantially less than 1.0.

---

## 7. Empirical Research: Penalties Observed in Practice

The source code reveals the structural mechanisms. Empirical research fills in the behavioural picture.

### 7.1 Link Penalty (Historical, Largely Removed October 2025)

The 2026 open-source code does not contain an explicit link penalty in the scoring formula. However:

- Pre-October 2025, [Buffer's analysis of 18.8 million X posts](https://buffer.com/resources/links-on-x/) found a **30-50% reach reduction** for posts containing external URLs.
- Non-Premium accounts posting links had **zero median engagement** from March 2025 onward.
- In October 2025, [X announced removal of algorithmic link penalties](https://tomorrowspublisher.today/content-creation/x-softens-stance-on-external-links/). Early data showed approximately 8x increase in link post reach.
- As of 2026, the structural penalty is gone from the scoring code, but the Grok model may still learn to suppress link posts indirectly if users historically engage less with them.

### 7.2 Hashtag Over-Use

No hashtag penalty exists in the source code. The observed penalty is indirect:

- [Posts with 1-2 relevant hashtags see 21% higher engagement](https://postowl.io/blog/twitter-hashtags-x-algorithm-2025/).
- [Posts with 5+ hashtags see a 40% engagement reduction](https://contentstudio.io/blog/twitter-hashtags) -- likely because the Grok model associates hashtag-heavy posts with spam patterns users historically disengage from (higher P(not_interested), higher P(mute)).

### 7.3 Sentiment and Combative Content (Grok Analysis)

The Grok transformer processes the **full text and media** of every post. From [empirical reporting](https://posteverywhere.ai/blog/how-the-x-twitter-algorithm-works):

- Positive and constructive messaging receives wider distribution.
- Negative and combative tones receive reduced visibility **even if engagement is high**.
- The mechanism is indirect: combative content generates higher P(block), P(mute), P(report) predictions, which the weighted scorer then penalises.

This is not a separate "sentiment penalty" -- it is the negative signals system working as designed. Content that provokes blocks/mutes/reports, even from a minority of viewers, gets scored down.

### 7.4 Shadowban Triggers (Rate-Limiting Penalties)

These are enforced outside the recommendation algorithm at the account/activity level:

- **Excessive actions**: >100 likes/hour, >50 retweets/hour, >30 replies/hour trigger spam detection.
- **Follow/unfollow cycling**: Following 100+ accounts/day or rapid follow/unfollow patterns can trigger a [48-72 hour shadowban](https://pixelscan.net/blog/twitter-shadowban-2026-guide/).
- **Mass unfollowing**: A surge of unfollows within a short period can trigger a [3-month visibility reduction](https://blog-content.circleboom.com/the-hidden-x-algorithm-tweepcred-shadow-hierarchy-dwell-time-and-the-real-rules-of-visibility/).
- **Dwell time decay**: If users consistently scroll past your posts in under 3 seconds, the model learns low dwell-time predictions for your content, reducing its score.

### 7.5 Coordinated Negative Actions

Multiple users blocking/muting the same account trains the model to predict higher P(block)/P(mute) for that account's content across all users. This creates a feedback loop: [coordinated blocking campaigns can artificially destroy an account's reach](https://blog-content.circleboom.com/the-hidden-x-algorithm-tweepcred-shadow-hierarchy-dwell-time-and-the-real-rules-of-visibility/) by poisoning the model's predictions.

### 7.6 Premium vs. Non-Premium Disparity

Not a penalty per se, but Premium accounts receive [a 4x visibility boost for in-network and 2x for out-of-network content](https://www.tweetarchivist.com/how-twitter-algorithm-works-2025). Non-Premium accounts operate at a structural disadvantage, making penalties relatively more devastating.

---

## 8. What Isn't in the Code (But Matters)

The open-source release excludes several components:

| Excluded | Label |
|----------|-------|
| `params.rs` | "Excluded from open source release for security reasons" |
| `clients/` | All external service clients (Phoenix prediction, Grok, Strato, etc.) |
| `util/` | Utility functions including `normalize_score()` and `score_normalizer` |

The `normalize_score()` function (called in `weighted_scorer.rs` line 22, after `offset_score()`) likely applies additional per-candidate normalisation. Without it, we cannot determine the exact final score distribution. The `offset_score()` analysis above represents the structural logic, not the final numerical output.

---

## 9. Summary: The Penalty Hierarchy

From most severe to least severe:

| Rank | Penalty | Mechanism | Reversibility |
|------|---------|-----------|---------------|
| 1 | **VF safety drop** | Post removed entirely (spam, violence, policy) | Irreversible for that post |
| 2 | **Block/mute filter** | Author's content removed from viewer's feed | Reversible by unblocking/unmuting |
| 3 | **Muted keyword filter** | Posts with matching text removed | Reversible by removing keyword |
| 4 | **Report prediction** | P(report) * REPORT_WEIGHT drags score toward zero | Persistent -- model must relearn |
| 5 | **Block prediction** | P(block) * BLOCK_AUTHOR_WEIGHT | Persistent -- model must relearn |
| 6 | **Mute prediction** | P(mute) * MUTE_AUTHOR_WEIGHT | Persistent -- model must relearn |
| 7 | **Not Interested prediction** | P(not_interested) * NOT_INTERESTED_WEIGHT | Persistent -- model must relearn |
| 8 | **Out-of-network penalty** | Score * OON_WEIGHT_FACTOR | Structural -- follow the author to bypass |
| 9 | **Author diversity decay** | Score * decay^(position) for repeated authors | Resets each feed request |
| 10 | **Rate-limit shadowban** | Account-level visibility reduction | 48-72 hours to 3 months |

---

## 10. Practical Implications

### For content creators

1. **Negative signals compound.** A post that generates even moderate P(not_interested) and P(mute) predictions will enter the compression branch and effectively die. It does not need to be reported -- just predicted as reportable.

2. **Your history trains the model against you.** Every block, mute, and report you have received becomes training data. The model generalises: if users who engage with longevity content also tend to mute your posts, the model will suppress your content for the entire longevity-interested cohort.

3. **Combative tone is penalised through the negative signals pathway**, not through a separate sentiment filter. Frame critiques constructively to avoid elevating P(block) and P(mute).

4. **The offset compression means there is no "so bad it underflows."** The worst possible negative score is bounded. But the bound is near zero, which means the content still gets ranked below essentially everything.

5. **Hard filters are absolute.** If users mute keywords that appear in your content, or block you, no amount of positive engagement from other users will make your content appear in their feed. Filters run before scoring.

6. **Out-of-network content faces double jeopardy**: stricter VF safety levels AND the OON_WEIGHT_FACTOR multiplier. Growing beyond your existing follower base requires consistently high positive predictions with near-zero negative predictions.

---

## Sources

**Source code**: [xai-org/x-algorithm](https://github.com/xai-org/x-algorithm) (Apache 2.0, January 2026)

**Empirical research**:
- [How the Twitter Algorithm Works in 2026 -- Tweet Archivist](https://www.tweetarchivist.com/how-twitter-algorithm-works-2025)
- [The Hidden X Algorithm: TweepCred, Shadow Hierarchy, Dwell Time -- Circleboom](https://blog-content.circleboom.com/the-hidden-x-algorithm-tweepcred-shadow-hierarchy-dwell-time-and-the-real-rules-of-visibility/)
- [How the Twitter/X Algorithm Works in 2026 (Source Code) -- PostEverywhere](https://posteverywhere.ai/blog/how-the-x-twitter-algorithm-works)
- [X Softens Stance on External Links -- Tomorrow's Publisher](https://tomorrowspublisher.today/content-creation/x-softens-stance-on-external-links/)
- [Do Posts with Links Affect Content Performance on X? -- Buffer](https://buffer.com/resources/links-on-x/)
- [Twitter Shadowban: Causes, Detection & Fixes (2026) -- Pixelscan](https://pixelscan.net/blog/twitter-shadowban-2026-guide/)
- [Twitter Hashtags in 2025: X Algorithm Data & Strategy -- PostOwl](https://postowl.io/blog/twitter-hashtags-x-algorithm-2025)
- [100+ Viral X Hashtags to Boost Reach in 2026 -- ContentStudio](https://contentstudio.io/blog/twitter-hashtags)
- [X's Algorithm Is Shifting to a Grok-Powered AI Model -- Social Media Today](https://www.socialmediatoday.com/news/x-formerly-twitter-switching-to-fully-ai-powered-grok-algorithm/803174/)
- [X/Twitter Algorithm Changes Timeline (2024-2026) -- Success On X](https://www.successonx.com/algorithm-changes)
- [I Read X's Open-Source Algorithm So You Don't Have To -- Medium](https://medium.com/@enjoykaz/i-read-xs-open-source-algorithm-so-you-don-t-have-to-86a9c8bba2a7)
- [X open sources its algorithm -- TechCrunch](https://techcrunch.com/2026/01/20/x-open-sources-its-algorithm-while-facing-a-transparency-fine-and-grok-controversies/)
