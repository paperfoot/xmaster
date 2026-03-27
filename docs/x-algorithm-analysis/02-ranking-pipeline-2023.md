# X/Twitter Ranking Pipeline (2023 Open-Source Release)

> Analysis of the actual ranking code from [twitter/the-algorithm](https://github.com/twitter/the-algorithm) and [twitter/the-algorithm-ml](https://github.com/twitter/the-algorithm-ml), open-sourced March 31, 2023.

---

## Table of Contents

1. [Pipeline Overview](#1-pipeline-overview)
2. [Light Ranker (EarlyBird)](#2-light-ranker-earlybird)
3. [Heavy Ranker (MaskNet)](#3-heavy-ranker-masknet)
4. [Engagement Weights and Scoring Formula](#4-engagement-weights-and-scoring-formula)
5. [Home Mixer Orchestration](#5-home-mixer-orchestration)
6. [Blue/Premium Boosts](#6-bluepremium-boosts)
7. [Heuristic Rescoring and Filters](#7-heuristic-rescoring-and-filters)
8. [Implications for Small Accounts](#8-implications-for-small-accounts)
9. [Key Source Files](#9-key-source-files)

---

## 1. Pipeline Overview

Every tweet that might appear on the "For You" timeline passes through a four-stage funnel:

```
Candidate Generation  (~1,500 tweets)
        |
  Light Ranker        (logistic regression in EarlyBird -- fast pre-filter)
        |
  Heavy Ranker        (48M-param MaskNet -- full neural scoring)
        |
  Heuristic Rescoring (boosts, diversity, fatigue, filters)
        |
  Final Timeline      (~50 tweets served)
```

The key orchestration layer is **Home Mixer** (`home-mixer/`), a Scala service that wires together candidate sources, feature hydration, ML scoring, and post-scoring heuristics.

---

## 2. Light Ranker (EarlyBird)

**Source:** `src/python/twitter/deepbird/projects/timelines/scripts/models/earlybird/`

### Architecture

- **Model type:** Logistic regression (binary classification)
- **Purpose:** Cheap pre-filter that evaluates thousands of candidates before the expensive heavy ranker sees them
- **Two separate models:**
  - `recap_earlybird` -- in-network tweets (from accounts you follow)
  - `rectweet_earlybird` -- out-of-network tweets (from UTEG recommendations)
- **Training framework:** Twitter's internal `twml` library via `train.py`
- **Status at time of release:** Several years old; the team noted plans for complete reconstruction

### Feature Sources

The light ranker ingests four feature categories:

| Source | Features | Examples |
|--------|----------|----------|
| **Index Ingester** | Static tweet metadata at creation time | URL presence, cards, quotes, tweet type (retweet/reply) |
| **Signal Ingester** | Real-time engagement via Heron topology | Retweet counts, favorites, replies |
| **User Table Features** | Per-user characteristics from user service | Account properties matched via author lookup |
| **Search Context Features** | Searcher-side context | Language, UI settings, timestamp |

### Static Features Used

- Tweet type flags: `IsRetweetFeature`, `InReplyToTweetIdFeature`
- Link/media presence
- Trend alignment scores
- Text quality scores from `TweetTextScorer`
- Health model outputs: toxicity score, block probability

### Integration with EarlyBird Search

EarlyBird is a **real-time search system built on Apache Lucene**. It maintains three index clusters:

| Cluster | Coverage |
|---------|----------|
| **Realtime** | Last 7 days of public tweets |
| **Protected** | Last 7 days of protected tweets |
| **Archive** | Historical tweets (up to 2 days prior) |

The light ranker runs inside EarlyBird's `ScoringModelsManager` (located in the `ml/` directory), scoring candidates as they are retrieved from the inverted index. This is what makes it fast -- scoring happens co-located with retrieval, not as a separate network hop.

---

## 3. Heavy Ranker (MaskNet)

**Source:** `the-algorithm-ml/projects/home/recap/`

### Architecture: Parallel MaskNet

The heavy ranker is a **48M-parameter multi-task neural network** using Parallel MaskNet architecture.

**Config file:** `projects/home/recap/config/local_prod.yaml`

```
Backbone: Parallel MaskNet
  - 4 mask blocks
  - Aggregation size: 1024 per block
  - Output size: 1024 per block
  - No layer normalization within blocks
  - Parallel processing enabled

MLP layer: 2048 units (single layer after mask block concatenation)

Featurization: DoubleNormLog
  - Clip magnitude: 5.0
  - Log transformation with double normalization
```

#### MaskBlock Internal Structure

Each `MaskBlock` (from `model/mask_net.py`) contains:

```python
class MaskBlock(torch.nn.Module):
    # 1. Optional LayerNorm(input_dim)
    # 2. Mask generation:
    #      Linear(mask_input_dim -> aggregation_size)
    #      -> ReLU
    #      -> Linear(aggregation_size -> input_dim)
    # 3. Element-wise product: input * mask
    # 4. Hidden layer: Linear(input_dim -> output_size)
    # 5. Output LayerNorm(output_size)
```

**Parallel mode operation:** All 4 MaskBlocks process identical inputs independently. Outputs are concatenated along dimension 1, producing a total dimension of 4 x 1024 = 4096, which then feeds into the 2048-unit MLP.

**Weight initialization:**
- Linear layers: Xavier uniform
- Bias: constant 0

#### Input Features

Approximately **~6,000 features per tweet**, assembled from:

| Feature Type | Count | Truncation |
|--------------|-------|------------|
| Continuous | 2,117 | Yes |
| Binary | 59 | Yes |
| Discrete | Variable | Truncated |

Features are sourced from 14 tweet feature hydrators (from `TweetypieStaticEntitiesFeatureHydrator`):

```
AuthorIdFeature              HasImageFeature
DirectedAtUserIdFeature      HasVideoFeature
InReplyToTweetIdFeature      IsRetweetFeature
InReplyToUserIdFeature       MentionScreenNameFeature
QuotedTweetIdFeature         MentionUserIdFeature
QuotedUserIdFeature          SourceTweetIdFeature
SourceUserIdFeature          ExclusiveConversationAuthorIdFeature
```

Plus embeddings from:
- **RealGraph** -- viewer-author relationship affinity scores
- **SimClusters** -- sparse topic embedding features
- **TwHIN** -- user and tweet embeddings from heterogeneous information network
- **CLIP** -- media understanding embeddings

#### Task-Specific Towers (13 Engagement Tasks)

Each tower is an identical MLP:

```
Layer 1: 256 units + BatchNorm(momentum=0.1)
Layer 2: 128 units
Output:  1 unit (sigmoid -> probability)
Pos weight: 1.0 for all tasks
```

Dropout varies by task:
- `is_negative_feedback_v2`: dropout = 0.1
- `is_report_tweet_clicked`: dropout = 0.2
- All others: no dropout

#### Training Configuration

```yaml
optimizer: Adam
  backbone_lr: 0.0001 (1000-step linear ramp)
  tower_lr: 0.0001-0.003 (1000-5000 step ramps per task)
  beta1: 0.95
  beta2: 0.999
  epsilon: 1.0e-07

batch_size: 128 (global)
eval_timeout: 7200 seconds
negative_downsampling: 0.000014 to 0.01 (per task)
positive_downsampling: 0.8387 to 1.0 (per task)
```

#### Forward Pass (from `model/entrypoint.py`)

```
1. Feature preprocessing (DoubleNormLog normalization)
2. Embedding computation (large + small embeddings)
3. Optional position debiasing
4. Concatenation -> shared MaskNet backbone
5. Per-task tower -> logits
6. Affine transformation -> adjusted logits
7. Sigmoid -> probabilities
8. Numeric calibration -> final P(engagement_i)
```

The model returns stacked logits, raw probabilities, and calibrated probabilities across all 13 tasks.

---

## 4. Engagement Weights and Scoring Formula

**Source:** `the-algorithm-ml/projects/home/recap/README.md`

### The Formula

```
score = SUM( weight_i * P(engagement_i) )
```

Where `P(engagement_i)` is the calibrated probability from the heavy ranker for each engagement type, and `weight_i` is the engagement weight.

### Engagement Weights (April 5, 2023)

| Engagement Type | Weight | Relative to Like | Variable Name |
|----------------|--------|-------------------|---------------|
| **reply_engaged_by_author** | **+75.0** | **150x** | `scored_tweets_model_weight_reply_engaged_by_author` |
| **reply** | **+13.5** | **27x** | `scored_tweets_model_weight_reply` |
| **good_profile_click** | **+12.0** | **24x** | `GoodProfileClickParam` |
| **good_click** (conversation click) | **+11.0** | **22x** | `GoodClickParam` |
| **good_click_v2** (2+ min dwell) | **+10.0** | **20x** | `GoodClickV2Param` |
| **retweet** | **+1.0** | **2x** | `RetweetParam` |
| **fav** (like) | **+0.5** | **1x (baseline)** | `FavParam` |
| **video_playback50** | **+0.005** | **0.01x** | `VideoPlayback50Param` |
| **negative_feedback_v2** | **-74.0** | **-148x** | `NegativeFeedbackV2Param` |
| **report** | **-369.0** | **-738x** | `ReportParam` |

#### What this means in practice

The weights were "originally set so that, on average, each weighted engagement probability contributes a near-equal amount to the score." Since replies are rare but weighted 27x a like, and likes are common but weighted at 0.5, the expected contribution balances out.

**Critical insight:** A single "report" (-369) destroys the equivalent of **738 likes**. A single "Show less often" click (-74) cancels **148 likes**.

### Additional Model Weight Parameters (from `HomeGlobalParams.scala`)

The code exposes many more weight knobs than the README documents. All default to 0.0 in the code, meaning they are set at runtime via config:

```
FavParam, RetweetParam, ReplyParam, GoodProfileClickParam,
VideoPlayback50Param, VideoQualityViewParam, VideoQualityViewImmersiveParam,
ReplyEngagedByAuthorParam, GoodClickParam, GoodClickV1Param, GoodClickV2Param,
TweetDetailDwellParam, ProfileDwelledParam, BookmarkParam, ShareParam,
ShareMenuClickParam, NegativeFeedbackV2Param, ReportParam,
WeakNegativeFeedbackParam, StrongNegativeFeedbackParam, DwellParam,
OpenLinkParam, ScreenshotParam, VideoWatchTimeMsParam,
VideoQualityWatchParam, Dwell0Param through Dwell4Param
```

The actual production values are injected at runtime and match the README table above.

### Model Debiasing Parameters

Separate from weights, the code has per-engagement **debias** parameters (all default 0.0, range -10000 to +10000):

```
Debias.FavParam, Debias.RetweetParam, Debias.ReplyParam,
Debias.DwellParam, Debias.GoodProfileClickParam,
Debias.VideoWatchTimeMsParam, Debias.ReplyEngagedByAuthorParam,
Debias.GoodClickV1Param, Debias.GoodClickV2Param,
Debias.BookmarkParam, Debias.ShareParam,
Debias.NegativeFeedbackV2Param, Debias.VideoQualityWatchParam
```

These allow additive correction to predicted probabilities before the weighted sum.

---

## 5. Home Mixer Orchestration

**Source:** `home-mixer/server/src/main/scala/com/twitter/home_mixer/`

### Pipeline Architecture

Home Mixer uses a nested pipeline structure:

```
HomeMixerRequest
  -> ScoredTweetsProductPipelineConfig (product = "ScoredTweets")
    -> ScoredTweetsRecommendationPipelineConfig
      -> Candidate Pipelines (parallel fetch)
        -> In-Network: TimelineRankerInNetworkCandidateSource
        -> Out-of-Network: TimelineRankerUtegCandidateSource
        -> Additional: CrMixer, FollowRecommendations
      -> Feature Hydration (~6,000 features per candidate)
      -> ML Scoring (Navi model server)
      -> Heuristic Rescoring
      -> Filtering
```

### Candidate Sources

| Source | Pipeline Class | Content |
|--------|---------------|---------|
| **In-Network** | `ScoredTweetsInNetworkCandidatePipelineConfig` | Tweets from followed accounts via Timeline Ranker Recycled |
| **UTEG (Out-of-Network)** | `ScoredTweetsUtegCandidatePipelineConfig` | Tweets liked/engaged by people in your graph |
| **CrMixer** | Separate pipeline | Content-based recommendations |
| **Follow Recommendations** | Separate pipeline | Who-to-follow suggestions |

### Feature Hydration

Before scoring, each candidate tweet gets enriched with features from:

```
Query-level features:
  - ServedTweetIdsFeature (previously shown tweets)
  - TimelineServiceTweetsFeature (backfill candidates)
  - SignupCountryFeature, SignupSourceFeature
  - ViewerAllowsForYouRecommendationsFeature
  - ServedAuthorIdsFeature
  - UserFollowersCountFeature

Candidate-level features:
  - RetweetSourceTweetFeatureHydrator (retweet metadata)
  - IsExtendedReplyFeatureHydrator (extended reply detection)
  - ReplyFeatureHydrator (reply enrichment)
  - RealGraphViewerRelatedUsersFeatureHydrator (social graph affinity)
  - TopicBasedRealTimeAggregateFeatureHydrator
```

### ML Scoring via Navi

The `NaviModelScorer` sends features to a gRPC prediction service:

```scala
// Batch size: 64 predictions per request
// Parallel conversion: 32 candidates per batch
// Model ID: "Home"
// Feature set: AllFeatures()

modelClient.getPredictions(records, commonRecord, modelId = Some("Home"))
```

The scorer computes the final weighted sum:

```
finalScore = SUM(predictedScoreFeature * modelWeightParam)
```

With separate handling for positive and negative weights before combination.

### Blending In-Network vs Out-of-Network

The out-of-network scaling is explicit in the code:

```scala
// OONTweetScalingScorer.scala
private val ScaleFactor = 0.75  // 25% penalty for out-of-network

// Applied when:
// 1. Tweet is NOT in-network (!InNetworkFeature)
// 2. Tweet is NOT a retweet (!IsRetweetFeature)
// Note: In-network retweets of OON tweets are exempt
```

This means out-of-network tweets start at a 25% disadvantage before any other heuristic is applied.

---

## 6. Blue/Premium Boosts

**Source:** Originally in `HomeGlobalParams.scala` (later removed from public repo after backlash)

### The Boost Parameters

```scala
// BlueVerifiedAuthorInNetworkMultiplierParam
// Default: 4.0
// Range: [0.1, 100.0]

// BlueVerifiedAuthorOutOfNetworkMultiplierParam
// Default: 2.0
// Range: [0.1, 100.0]
```

| Subscriber Status | In-Network | Out-of-Network |
|-------------------|-----------|----------------|
| **Blue/Premium** | **4x boost** | **2x boost** |
| **Free account** | 1x (baseline) | 0.75x (OON penalty) |

**Effective gap:** A Premium subscriber's tweet gets a **4x** in-network boost while a free account's out-of-network tweet gets a **0.75x** scale -- that is a **5.3x advantage** before any engagement signal is considered.

### Tweepcred (Account Quality Score)

**Source:** `src/scala/com/twitter/graph/batch/job/tweepcred/`

An account-level authority score from 0-100, derived from a modified PageRank:

```
If isVerified: score = 100 (automatic max)
If suspended: score = 0

Otherwise computed from:
  - Follower-to-following ratio
  - Account age
  - Interaction quality
  - Device validity: +0.5 additive bonus
```

**Critical threshold at Tweepcred 65:**
- Below 65: Maximum **3 tweets** considered by the ranking algorithm
- Above 65: **No limit** on tweets considered

This is a hard gate. A small account with Tweepcred below 65 can only surface 3 tweets into the ranking pipeline per cycle, regardless of content quality.

---

## 7. Heuristic Rescoring and Filters

**Source:** `home-mixer/server/src/main/scala/com/twitter/home_mixer/`

### Rescoring Chain (Applied Sequentially)

The `HeuristicScorer` chains these rescorers in order, each multiplying the score:

```
1.  RescoreOutOfNetwork (0.75x penalty)
2.  RescoreReplies (0.75x penalty)
3.  MTL Normalization (alpha/100, beta, gamma params)
4.  Content Exploration Diversity
5.  Deep Retrieval Signals (standard + evergreen + cross-border)
6.  Author-Based Signals
7.  Impressed Author Decay
8.  Impressed Media Cluster Rescoring
9.  Impressed Image Cluster Rescoring
10. Candidate Source Diversity
11. Grok Slop Scoring
12. Feedback Fatigue Penalties
13. Multimodal Embedding Signals
14. Live Content Boost
15. Control AI Rescorers
```

**Score formula:** `updatedScore = score * scaleFactor` (cumulative product of all rescorer multipliers, with epsilon protection for negative scores)

### Author Diversity Decay

Prevents timeline domination by a single author:

```scala
// ScoredTweetsParam.scala
AuthorDiversityDecayFactor     = 0.5   // Each subsequent tweet by same author: 50% of previous
AuthorDiversityFloor           = 0.25  // Minimum multiplier (never below 25%)

// In-network specific:
AuthorDiversityInNetworkDecayFactor  = 0.5
AuthorDiversityInNetworkFloor        = 0.25

// Out-of-network specific:
AuthorDiversityOutNetworkDecayFactor = 0.5
AuthorDiversityOutNetworkFloor       = 0.25

// Small accounts (few follows):
SmallFollowGraphAuthorDiversityDecayFactor = 0.5
SmallFollowGraphAuthorDiversityFloor       = 0.25
```

**Effect on sequence:** If you post 4 tweets, they receive multipliers of: 1.0, 0.5, 0.25, 0.25 (capped at floor).

### Candidate Source Diversity

```scala
CandidateSourceDiversityDecayFactor = 0.9  // 10% penalty per duplicate source
CandidateSourceDiversityFloor       = 0.8  // Never below 80%
```

### Feedback Fatigue

**Source:** `FeedbackFatigueScorer.scala`

When a user clicks "Show less often" on someone's content:

```
Duration: 140 days
Decay: Linear over 4 steps (35 days each)
Multiplier range: 0.2 (max penalty) to 1.0 (no penalty)
Increment: +0.2 per 35-day period

Final score = score * (author_mult * liker_mult * follower_mult * retweeter_mult)
```

Four separate fatigue tracks are multiplied together:
1. **Tweet author** -- direct "See Fewer" on this author
2. **Likers** -- discount if all engagers are in the penalty map
3. **Followers** -- maximum discount when no likers are present
4. **Retweeters** -- applies only to retweets

### Tweet Relevancy Decay

```
Half-life: 360 minutes (6 hours)
Decay rate: 0.003
Minimum score: 0.6

Score multiplier = max(0.6, e^(-0.003 * age_in_minutes))
```

A tweet loses 50% of its ranking relevance every 6 hours.

### Content Filters

```scala
// Negative score thresholds:
NegativeScoreConstantFilterThresholdParam = 0.001
NegativeScoreNormFilterThresholdParam     = 0.15

// Slop filtering (low-quality content):
SlopMaxScore       = 0.3   // Score threshold
SlopMinFollowers   = 100   // Author follower minimum

// Video duration:
MinVideoDurationThresholdParam = 0 ms
MaxVideoDurationThresholdParam = 604800000 ms (7 days)

// Historical dedup window:
DedupHistoricalEventsTimeWindowParam = 43200000 ms (12 hours)

// Control AI (user "Show More"/"Show Less" signals):
ControlAiShowLessScaleFactor  = 0.05  // 95% reduction
ControlAiShowMoreScaleFactor  = 20.0  // 20x boost
```

### Heavy Ranker Selection Gate

```scala
IsSelectedByHeavyRankerCountParam = 100  // max 100 tweets pass to final ranking
```

Only the top 100 candidates (by heavy ranker score) proceed to heuristic rescoring.

---

## 8. Implications for Small Accounts

### The Compounding Disadvantage Stack

A small, free account faces multiple simultaneous penalties:

| Factor | Impact | Source |
|--------|--------|--------|
| **No Blue boost** | 1x vs 4x in-network | `BlueVerifiedAuthorInNetworkMultiplierParam` |
| **Tweepcred < 65** | Max 3 tweets in pipeline | Tweepcred threshold gate |
| **Low follower count** | Fewer in-network impressions | Graph size limits candidate pool |
| **OON penalty** | 0.75x for non-followers | `OONTweetScalingScorer: ScaleFactor = 0.75` |
| **Author diversity** | 0.5x decay per tweet | `AuthorDiversityDecayFactor = 0.5` |
| **Slop filter risk** | Filtered if < 100 followers | `SlopMinFollowers = 100` |
| **No engagement flywheel** | Low P(engagement) predictions | MaskNet predicts based on historical signals |

### Quantified Disadvantage

**Scenario:** Small free account (500 followers, Tweepcred 50) vs Premium account (50K followers, Tweepcred 85) posting identical content.

For an in-network viewer:
```
Premium:  score * 4.0 (Blue boost) * 1.0 (no OON penalty)  = 4.0x
Small:    score * 1.0 (no boost)   * 1.0 (in-network)      = 1.0x
Gap: 4x
```

For an out-of-network viewer:
```
Premium:  score * 2.0 (Blue OON boost)  = 2.0x
Small:    score * 0.75 (OON penalty)    = 0.75x
Gap: 2.67x
```

But it gets worse. The heavy ranker's `P(engagement_i)` predictions are trained on historical data. An account with 500 followers has lower base rates for all engagements compared to a 50K-follower account. The model learns this and predicts lower probabilities, which then get multiplied by the weights:

```
Small account P(reply)    ~ 0.001  -> weighted: 0.001 * 13.5 = 0.0135
Large account P(reply)    ~ 0.01   -> weighted: 0.01  * 13.5 = 0.135

10x gap before any boost is applied
```

### What Actually Drives Ranking for Small Accounts

Given the weight table, a small account's best strategy is to optimize for the highest-weighted engagement types:

1. **Reply-engaged-by-author (+75.0):** Post content that generates replies, then reply back to every comment. This single behavior is worth **150 likes**. It is by far the most valuable engagement signal.

2. **Reply (+13.5):** Content that provokes replies is worth **27 likes** per reply. Questions, controversial takes, and "fill in the blank" formats trigger this.

3. **Good profile click (+12.0):** Content that makes people visit your profile -- achieved through an interesting bio, consistent niche, and novel perspective.

4. **Good click / conversation click (+11.0):** Tweets that people expand to read more (thread starters, images with text that requires expansion).

5. **Avoid negative feedback at all costs:** A single "Show less" (-74) erases 148 likes. A single report (-369) erases 738 likes. Polarizing content that generates both engagement AND negative feedback can be net-negative.

### The Tweepcred Trap

With Tweepcred below 65, only 3 tweets enter the pipeline per cycle. This means:

- Posting 10x per day wastes 7 tweets (they never reach the ranker)
- The 3 tweets that do enter must each independently generate enough engagement to overcome the boost gap
- Account age, follower ratio, and device legitimacy are the levers that move Tweepcred
- Verified (Blue) accounts get Tweepcred = 100 automatically

### The 6-Hour Window

With a tweet half-life of 360 minutes:

```
0 hours:  100% relevancy
6 hours:   50% relevancy
12 hours:  25% relevancy
24 hours:   6% relevancy (near-zero)
```

For small accounts, timing matters disproportionately. A tweet that does not generate engagement within 6 hours is effectively dead. Posting when your audience is active is not optional -- it is the difference between getting ranked and being invisible.

---

## 9. Key Source Files

### Heavy Ranker (ML)

| File | Purpose |
|------|---------|
| `the-algorithm-ml/projects/home/recap/README.md` | Engagement weights, scoring formula |
| `the-algorithm-ml/projects/home/recap/config/local_prod.yaml` | MaskNet architecture config |
| `the-algorithm-ml/projects/home/recap/model/mask_net.py` | MaskBlock/MaskNet implementation |
| `the-algorithm-ml/projects/home/recap/model/config.py` | Model config classes (MaskNetConfig, TaskModel, etc.) |
| `the-algorithm-ml/projects/home/recap/model/entrypoint.py` | MultiTaskRankingModel construction and forward pass |
| `the-algorithm-ml/projects/home/recap/model/model_and_loss.py` | Loss computation and stratification |
| `the-algorithm-ml/projects/home/recap/data/config.py` | Data pipeline config, sampling rates |

### Light Ranker

| File | Purpose |
|------|---------|
| `the-algorithm/src/python/twitter/deepbird/projects/timelines/scripts/models/earlybird/` | Light ranker models |
| `the-algorithm/src/java/com/twitter/search/earlybird/` | EarlyBird search system, Lucene-based indexing |

### Home Mixer (Orchestration)

| File | Purpose |
|------|---------|
| `the-algorithm/home-mixer/server/.../ScoredTweetsProductPipelineConfig.scala` | Top-level pipeline wiring |
| `the-algorithm/home-mixer/server/.../ScoredTweetsParam.scala` | All tunable parameters (diversity, decay, scale factors) |
| `the-algorithm/home-mixer/server/.../param/HomeGlobalParams.scala` | Global params (Blue boosts, model weights, filters) |
| `the-algorithm/home-mixer/server/.../scorer/NaviModelScorer.scala` | ML model gRPC client, weighted score computation |
| `the-algorithm/home-mixer/server/.../scorer/HeuristicScorer.scala` | 15-step rescoring chain |
| `the-algorithm/home-mixer/server/.../scorer/OONTweetScalingScorer.scala` | Out-of-network 0.75x penalty |
| `the-algorithm/home-mixer/server/.../scorer/FeedbackFatigueScorer.scala` | 140-day fatigue decay |
| `the-algorithm/home-mixer/server/.../feature_hydrator/TweetypieStaticEntitiesFeatureHydrator.scala` | 14 tweet feature extractors |
| `the-algorithm/home-mixer/server/.../feature_hydrator/RealGraphViewerRelatedUsersFeatureHydrator.scala` | Social graph affinity scoring |
| `the-algorithm/home-mixer/server/.../candidate_pipeline/ScoredTweetsInNetworkCandidatePipelineConfig.scala` | In-network source |
| `the-algorithm/home-mixer/server/.../candidate_pipeline/ScoredTweetsUtegCandidatePipelineConfig.scala` | Out-of-network source (UTEG) |

### Account Quality

| File | Purpose |
|------|---------|
| `the-algorithm/src/scala/com/twitter/graph/batch/job/tweepcred/` | Tweepcred (PageRank-derived account score) |

---

## Appendix: Architecture Diagram

```
                              HOME MIXER
                                  |
                 +----------------+----------------+
                 |                                 |
         In-Network Source                 Out-of-Network Source
    (TimelineRankerRecycled)              (UTEG, CrMixer)
                 |                                 |
                 +---------> Candidates <----------+
                              (~1,500)
                                  |
                          Feature Hydration
                         (~6,000 features/tweet)
                                  |
                     +------------+------------+
                     |                         |
              Light Ranker               (bypassed for
           (EarlyBird logistic           some sources)
             regression)
                     |
                     v
              Heavy Ranker
         (48M-param MaskNet)
         4 mask blocks x 1024
         13 task towers x [256,128,1]
                     |
                     v
             Weighted Score
    SUM(weight_i * P(engagement_i))
                     |
                     v
          Heuristic Rescoring
     (15 sequential multipliers)
        - Blue boost (4x/2x)
        - OON penalty (0.75x)
        - Author diversity (0.5x decay)
        - Feedback fatigue (0.2x-1.0x)
        - Relevancy decay (6hr half-life)
        - Slop filter
                     |
                     v
         Top 100 -> Final Timeline
              (~50 tweets served)
```

---

*Analysis based on source code from [twitter/the-algorithm](https://github.com/twitter/the-algorithm) (commit history as of April 2023) and [twitter/the-algorithm-ml](https://github.com/twitter/the-algorithm-ml). Note: Twitter has continued modifying the algorithm after open-sourcing; the 2023 snapshot may not reflect current production behavior.*
