# X/Twitter Recommendation Algorithm: Candidate Generation Pipeline (2023 Open-Source Release)

**Source**: [twitter/the-algorithm](https://github.com/twitter/the-algorithm) (released March 31, 2023)
**Scope**: This document covers the candidate generation stage only -- the first phase of the For You timeline pipeline that retrieves ~1,500 tweet candidates from a pool of hundreds of millions before ranking.

---

## Table of Contents

1. [Pipeline Overview](#1-pipeline-overview)
2. [SimClusters v2](#2-simclusters-v2)
3. [Real Graph](#3-real-graph)
4. [EarlyBird](#4-earlybird)
5. [GraphJet / UTEG](#5-graphjet--uteg)
6. [CrMixer](#6-crmixer)
7. [TwHIN](#7-twhin)
8. [Impact on Small Accounts (~100 Followers)](#8-impact-on-small-accounts-100-followers)
9. [Appendix: Key File Paths](#9-appendix-key-file-paths)

---

## 1. Pipeline Overview

The For You timeline retrieves approximately **1,500 candidate tweets** per request, split roughly:

- **~750 in-network** (from accounts you follow) -- served primarily by **EarlyBird**
- **~750 out-of-network** (from accounts you do not follow) -- served by **CrMixer**, which orchestrates **SimClusters**, **TwHIN**, and **GraphJet/UTEG**

These candidates then pass through a Light Ranker (logistic regression inside EarlyBird) and a Heavy Ranker (~48M parameter MaskNet) before final timeline assembly.

**Source**: [Twitter Engineering Blog](https://blog.x.com/engineering/en_us/topics/open-source/2023/twitter-recommendation-algorithm)

```
User Request
    |
    v
+------------------+     +------------------+     +------------------+
|   EarlyBird      |     |   CrMixer        |     |   Home Mixer     |
|   (in-network    |     |   (out-of-network |     |   (assembly,     |
|    ~50% tweets)  |     |    ~50% tweets)   |     |    ranking,      |
+------------------+     +------------------+     |    filtering)    |
    |                         |                    +------------------+
    |    SimClusters ANN      |
    |    TwHIN ANN            |
    |    GraphJet/UTEG        |
    |    EarlyBird OON        |
    +-------------------------+
```

---

## 2. SimClusters v2

**Code path**: `src/scala/com/twitter/simclusters_v2/`

SimClusters is the backbone of X's out-of-network recommendations. It maps every user and tweet into the same sparse, interpretable embedding space built on overlapping communities.

### 2.1 Community Detection (KnownFor Matrix)

**What it does**: Identifies ~145,000 communities from ~20 million producers using the follow graph.

**Algorithm**: Custom Sparse Binary Factorization (SBF) with Metropolis-Hastings sampling.

**Key file**: `src/scala/com/twitter/simclusters_v2/scalding/update_known_for/UpdateKnownFor20M145K2020.scala`

**Parameters from source code**:

| Parameter | Value | Source Variable |
|-----------|-------|-----------------|
| Producers indexed | 20,000,000 | `topK = 20_000_000` |
| Communities detected | ~145,000 | `k` parameter |
| Min active followers to qualify | 400 | `minActiveFollowers = 400` |
| Max neighbors per producer | 400 | `maxNeighbors = 400` |
| Clustering epochs | 3 (scheduled), 4 (adhoc) | `maxEpochs = 3` |
| Edge weight coefficient | 10.0 | `wtCoeff = 10.0` |

**Edge weight formula** (when `squareWeights` is enabled):

```
edge_weight = cosine_similarity^2 * 10.0
```

This squaring emphasizes strong follow-graph similarity and suppresses noise.

**Process**:

1. Compute cosine similarity between producers based on shared followers
2. Build producer-producer similarity graph with weighted edges
3. Run SBF + Metropolis-Hastings to assign each producer to at most one community
4. Output the **KnownFor matrix (V)** -- maximally sparse, one cluster per producer

**Key runner**: `UpdateKnownForSBFRunner.runUpdateKnownFor()`

```scala
// From UpdateKnownForSBFRunner.scala
// Algorithm flow:
// 1. Map 20M users to integer IDs (0..20M) for efficiency
// 2. Load neighbor arrays + edge weights into Graph object
// 3. Initialize SparseBinaryMatrix from previous week's assignments
// 4. Users without prior assignment -> random assignment to empty clusters
// 5. Run MHAlgorithm.optimize() for maxEpochs iterations
// 6. MHAlgorithm.heuristicallyScoreClusterAssignments()
// 7. When user has multiple assignments (~0.1% of cases),
//    select the maximum-scored cluster
```

**Data source**: Follow similarity stored at:
```
/atla/proc/user/cassowary/manhattan_sequence_files/approximate_cosine_similarity_follow
```

### 2.2 InterestedIn Computation

**What it does**: Computes what topics/communities each consumer is interested in by multiplying the follow graph by the KnownFor matrix.

**Key file**: `src/scala/com/twitter/simclusters_v2/scalding/InterestedInFromKnownFor.scala`

**Core formula**:

```
InterestedIn(user) = FollowGraph(user) x KnownFor(V)
```

For each user, the system examines who they follow, looks up those producers' KnownFor clusters, and aggregates signals.

**Parameters from source code**:

| Parameter | Value | Purpose |
|-----------|-------|---------|
| `socialProofThreshold` | **2** | Must follow/fav >= 2 users in a cluster to register interest |
| `maxClustersPerUser` | **50** (production), 20 (adhoc) | Max clusters retained per user |
| Fav score half-life | **100 days** | Decay for favorite-based signals |

**Scoring components per user-cluster pair**:

```scala
// From InterestedInFromKnownFor.scala

// 1. Follow Score: binary * knownForWeight
followScore = (if followed then 1.0 else 0.0) * knownForScore

// 2. Fav Score: decay-adjusted favorites * knownForWeight
favScore = srcWithWeights.favScoreHalfLife100Days.getOrElse(0.0) * knownForScore

// 3. Log Fav Score: log-transformed favorites * knownForWeight
logFavScore = srcWithWeights.logFavScore.getOrElse(0.0) * knownForScore

// 4. Producer-Normalized variants: divided by L2 norm of neighbor engagement
```

**Cluster-level L2 normalization**:

```scala
// attachNormalizedScores function
followNorm = math.sqrt(sumOfSquaredFollowScores_across_all_users_in_cluster)
normalizedFollowScore = followScore / followNorm
// NaN values converted to 0.0 via ifNanMake0()
```

**Ranking for truncation** (when user exceeds 50 clusters):

```scala
// Sort descending by this tuple:
(-favScore, -logFavScore, -followScore,
 -logFavScoreClusterNormalized, -followScoreProducerNormalized)
// Favorites dominate over follows in cluster retention
```

### 2.3 Real-Time Tweet Embeddings

**What it does**: When a user favorites a tweet, the tweet's embedding is updated by adding the favoriter's InterestedIn vector. This happens in real-time via a Heron/Storm streaming job.

**Key files**:
- `src/scala/com/twitter/simclusters_v2/summingbird/storm/TweetJobRunner.scala`
- `src/scala/com/twitter/simclusters_v2/summingbird/storm/TweetJob.scala`

**Process**:

```
1. User favorites tweet
2. Filter: userId != tweetUserId (no self-favorites)
3. Filter: isTweetTooOld() check
4. Fetch user's InterestedIn (ClustersUserIsInterestedIn)
5. Extract topClustersWithScores() -- typically ~25 clusters
6. Apply favScoreThresholdForUserInterest filter
7. Add user's cluster scores to tweet's embedding via monoid aggregation
8. Store in 3 caches:
   - Tweet -> top-K clusters
   - Cluster -> top-K tweets
   - Cluster -> top-K tweets (light variant)
```

**Topology configuration** (from `TweetJobRunner.scala`):

| Setting | Value |
|---------|-------|
| Total workers | 150 |
| Source workers per node | 1 |
| Flatmap workers per node | 3 |
| Summer workers per node | 3 |
| Source RAM | 8 GB |
| Processing RAM | 4 GB + 20% CPU overhead |
| Message timeout | 30 seconds |
| Scoring summer parallelism | 4x other stages |

**Critical insight**: Tweet embeddings start as empty vectors and grow organically as users engage. A tweet that gets no favorites has no SimClusters embedding and is invisible to this retrieval path.

### 2.4 SimClusters ANN Candidate Source

**Key file**: `src/scala/com/twitter/simclusters_v2/candidate_source/SimClustersANNCandidateSource.scala`

The ANN (Approximate Nearest Neighbor) service takes a user's InterestedIn embedding, finds tweets with high dot-product similarity, and returns scored candidates.

```scala
// From SimClustersANNSimilarityEngine.scala
// Scores come directly from the ANN service:
TweetWithScore(candidate.tweetId, candidate.score)
// No custom re-scoring -- trusts upstream similarity calculation
```

**For a small account**: If your tweets get even a few favorites from users whose InterestedIn vectors span diverse clusters, your tweet embedding grows and becomes retrievable by more users. The threshold is getting that initial engagement.

---

## 3. Real Graph

**Code path**: `src/scala/com/twitter/interaction_graph/`

Real Graph models the strength of relationships between user pairs. It powers seed user selection for both in-network and out-of-network candidate generation.

### 3.1 Edge Features

**Key file**: `src/scala/com/twitter/interaction_graph/scio/agg_all/InteractionGraphAggregationTransform.scala`

Each directed edge between two users tracks:

**Public engagements**:
- Favorites given/received
- Retweets given/received
- Follows (follow/unfollow events)

**Private engagements**:
- Profile views
- Tweet clicks
- Address book membership

**Node features** (per user):
- Retweets in last week
- Follower count
- Demographics
- PageRank (via TweepCred)

### 3.2 Decay Function

**Key file**: `src/scala/com/twitter/interaction_graph/scio/agg_all/InteractionGraphAggregationConfig.scala`

```scala
object InteractionGraphScoringConfig {
  val ALPHA = 1.0
  val ONE_MINUS_ALPHA = 0.955
}
```

This implements an **exponentially weighted moving average** where:

```
new_score = ALPHA * today_interactions + ONE_MINUS_ALPHA * previous_score
          = 1.0 * today + 0.955 * previous
```

The decay factor of **0.955** gives a **half-life of approximately 7 days**:

```
0.955^n = 0.5
n = log(0.5) / log(0.955) = ~15 days to reach 50%
```

(The README states "alpha such that the half life of weights is 7 days" -- the discrepancy suggests the decay may be applied twice per day or the formula accounts for additional normalization.)

### 3.3 Aggregation Pipeline

**Key file**: `src/scala/com/twitter/interaction_graph/scio/agg_all/InteractionGraphAggregationJob.scala`

The daily Dataflow job:
1. Takes yesterday's aggregated history
2. Adds today's interaction counts
3. Applies decay to historical weights
4. Joins with BQML-predicted interaction scores
5. Outputs updated edge weights as timeline features
6. Post-processing selects top-K edges per user using priority queues (`maxDestinationIds`)

### 3.4 ML Prediction Model (BQE)

**Code path**: `src/scala/com/twitter/interaction_graph/bqe/`

A gradient boosting classifier trained in BigQuery predicts interaction likelihood:

```
Features per edge:
- Number of tweets
- Number of follows
- Number of favorites
- Other behavioral metrics

Labels:
- 1 = interaction occurred in window
- 0 = no interaction

Split: by source user ID
```

### 3.5 TweepCred (PageRank)

**Code path**: `src/scala/com/twitter/graph/batch/job/tweepcred/`

TweepCred applies PageRank to the social graph with post-processing:

**Algorithm**:
1. `PreparePageRankData` -- builds interaction graph, initializes scores
2. `UpdatePageRank` -- iterates until convergence
3. Post-processing: raw PageRank -> logarithmic scaling -> byte scores (0-100)
4. **Penalty**: Users with low followers but high following get their PageRank divided down

**Critical threshold**:

```
if (tweepcred < 65):
    max_tweets_considered = 3  // Algorithm only looks at 3 most recent tweets
else:
    max_tweets_considered = unlimited
```

This is one of the most impactful thresholds for small accounts. A TweepCred under 65 means most of your tweets are never even evaluated.

---

## 4. EarlyBird

**Code path**: `src/java/com/twitter/search/earlybird/`

EarlyBird is X's real-time search engine built on Apache Lucene. It handles approximately **50% of in-network candidates** and also serves out-of-network retrieval for CrMixer.

### 4.1 Architecture

Three separate clusters:
- **Realtime cluster**: Last 7 days of tweets
- **Protected cluster**: Last 7 days of protected tweets
- **Archive cluster**: All historical tweets

Each cluster maintains:
- **Inverted index**: Term -> list of Doc IDs
- **Postings lists**: Optimized storage of Doc ID lists
- **Column Stride Fields (CSF)**: Per-document features for scoring

### 4.2 Scoring Function

**Key files**:
- `src/java/com/twitter/search/earlybird/search/relevance/scoring/LinearScoringFunction.java`
- `src/java/com/twitter/search/earlybird/search/relevance/LinearScoringData.java`

The scoring formula is a linear combination:

```java
score = BASE_SCORE + sum(weight[i] * feature[i])
// where BASE_SCORE = 0.0001
```

**Features from `LinearScoringData.java`**:

```java
// Engagement signals (log2-transformed)
retweetCountPostLog2         // log2(retweet_count)
favCountPostLog2             // log2(favorite_count)
replyCountPostLog2           // log2(reply_count)
embedsImpressionCount        // raw embed impressions
videoViewCount               // raw video views

// Temporal decay
tweetAgeInSeconds            // age of tweet
ageDecayMult                 // computed decay multiplier

// Social proof signals
isFollow                     // viewer follows author
isTrusted                    // author in trusted circle
isDirectFollow               // direct follow vs. list follow
isSelfTweet                  // viewer is the author

// Author quality
userRep                      // author reputation score (TweepCred-derived)
isVerified                   // legacy verified
isBlueVerified               // Twitter Blue subscriber

// Content signals
hasUrl, hasImage, hasVideo, hasNews
hasCard, hasQuote, hasHashtag, hasTrend
isNativeImage, isConsumerVideo, isProVideo

// Health model scores
toxicityScore, pBlockScore, spammyTweetScore
reportedTweetScore, contentSpamScore

// Constants
NO_BOOST_VALUE = 1.0f
UNSET_SIGNAL_VALUE = -999
MAX_OFFLINE_EXPERIMENTAL_FIELDS = 5
SKIP_HIT = -Float.MAX_VALUE  // marks docs to rank below everything
```

### 4.3 Age Decay

**Decay parameters** (from `ThriftAgeDecayRankingParams`):

| Parameter | Value |
|-----------|-------|
| Decay rate | **0.003** |
| Half-life | **360 minutes** (6 hours) |
| Minimum age decay score | **0.6** |

**Formula**:

```
ageDecayMult = max(0.6, exp(-0.003 * tweetAgeMinutes / 360))
```

A tweet 6 hours old retains ~50% of its age score. A tweet 24 hours old retains the floor of 0.6 (60%).

### 4.4 Boost Multipliers

From the open-source code and analyses:

| Signal | Boost |
|--------|-------|
| Image or video present | **2x** |
| Twitter Blue (in-network) | **4x** |
| Twitter Blue (out-of-network) | **2x** |
| Self-tweet | configurable boost |
| Trusted circle | configurable boost |

### 4.5 Light Ranker

**Key file**: `src/python/twitter/deepbird/projects/timelines/scripts/models/earlybird/README.md`

Two logistic regression variants:
- **`recap_earlybird`** -- in-network tweets (from followed accounts)
- **`rectweet_earlybird`** -- out-of-network tweets (from UTEG)

**Feature sources**:
1. **Index Ingester**: Static metadata (URL presence, quote status, language)
2. **Signal Ingester**: Real-time engagement (retweets, favorites, replies) via Heron topology
3. **User Table Features**: Per-author stats from user service
4. **Search Context Features**: Searcher's language, timestamp

The README notes this model was "trained several years ago" and Twitter was planning to rebuild it entirely.

### 4.6 Social Graph Compression

EarlyBird uses **Bloom filters** for social graph lookups:

```
// Check if viewer follows tweet author without loading full follow graph
// Probabilistic data structure -- false positives possible, no false negatives
bloomFilter.mightContain(authorId)
```

This enables fast social proof checks (is this tweet from someone I follow?) without requiring full graph traversal per candidate.

---

## 5. GraphJet / UTEG

**Code path**: `cr-mixer/server/src/main/scala/com/twitter/cr_mixer/candidate_generation/UtegTweetCandidateGenerator.scala`

**External repo**: [twitter/GraphJet](https://github.com/twitter/GraphJet)

GraphJet maintains a real-time bipartite graph between users and tweets entirely in-memory on a single server. UTEG (User Tweet Entity Graph) is the production deployment. It powers about **15% of For You tweets** (roughly **30% of out-of-network content**).

### 5.1 Bipartite Graph Structure

```
Users (left nodes) <---engagement edges---> Tweets (right nodes)

Edge types:
- Favorite
- Retweet
- Reply
- Quote

Temporal window: rolling (recent interactions only)
Processing: real-time streaming, not batch
```

### 5.2 SALSA Random Walk

**Algorithm**: SALSA (Stochastic Approach for Link-Structure Analysis)

```
1. Start from seed user set (viewer's "circle of trust")
2. Walk to tweets those users engaged with (user -> tweet edges)
3. Walk back to users who also engaged with those tweets (tweet -> user edges)
4. Repeat for multiple iterations
5. Tweets with highest authority scores become candidates

Node classification:
- Hubs (left/users): vertices with large out-degree
- Authorities (right/tweets): vertices with large in-degree
```

The walk discovers tweets that are popular among users similar to the viewer, even if the viewer has never interacted with those authors.

### 5.3 Circle of Trust

The "circle of trust" is computed via **personalized PageRank** from the viewer's social graph. It represents the most relevant users to seed the SALSA walk.

Common seed set: Users the viewer has recently engaged with (favorites, retweets, replies), weighted by Real Graph edge scores.

### 5.4 UTEG in CrMixer

**Key file**: `cr-mixer/server/src/main/scala/com/twitter/cr_mixer/similarity_engine/UserTweetEntityGraphSimilarityEngine.scala`

```scala
// From UserTweetEntityGraphSimilarityEngine.scala

// Social proof configuration:
MaxUserSocialProofSize = 10   // max users as proof per tweet
MaxTweetSocialProofSize = 10  // max tweets as proof per user
MinUserSocialProof = 1        // minimum 1 user must have engaged

// Default social proof type:
SocialProofType.Favorite      // favorites are primary signal

// Display context:
TweetEntityDisplayLocation.HomeTimeline
RecommendationType.Tweet
```

### 5.5 UTEG Candidate Generation Pipeline

```scala
// From UtegTweetCandidateGenerator.scala

// 1. Fetch seed users via Real Graph
seeds = realGraphInSourceGraphFetcher.get(FetcherQuery...)

// 2. Retrieve candidate tweets via UTEG engine
candidates = UserTweetEntityGraphSimilarityEngine.get(query)

// 3. Apply sequential filters
filtered = utegFilterRunner.runSequentialFilters(candidates)

// 4. Convert to ranked results with similarity scores
ranked = filtered.map(c => TweetWithScoreAndSocialProof(
    tweetId = c.tweetId,
    score = c.score,
    socialProofByType = c.socialProofByType
))

// 5. Take top results
result = ranked.take(query.maxNumResults)
```

---

## 6. CrMixer

**Code path**: `cr-mixer/server/src/main/scala/com/twitter/cr_mixer/`

CrMixer (Candidate Retrieval Mixer) is the coordination layer for all out-of-network candidate generation. It does not implement recommendation logic itself -- it orchestrates downstream services.

### 6.1 Pipeline Architecture

**Key file**: `cr-mixer/server/src/main/scala/com/twitter/cr_mixer/candidate_generation/CrCandidateGenerator.scala`

Seven sequential stages:

```
1. Source Signal Fetching     -> SourceInfoRouter (USS + FRS)
2. Candidate Generation       -> CandidateSourcesRouter
3. Pre-Rank Filtering         -> PreRankFilterRunner
4. Interleaving/Blending      -> SwitchBlender
5. Ranking                    -> SwitchRanker
6. Post-Rank Filtering        -> PostRankFilterRunner
7. Truncation                 -> query.maxNumResults
```

### 6.2 Candidate Sources

**Key file**: `cr-mixer/server/src/main/scala/com/twitter/cr_mixer/candidate_generation/CandidateSourcesRouter.scala`

Sources are enabled/disabled via feature flags, not hardcoded budgets:

| Source | Feature Flag | Signal Type |
|--------|-------------|-------------|
| SimClusters (LogFav) | `EnableLogFavBasedSimClustersTripParam` | Favorite history |
| SimClusters (Follow) | `EnableFollowBasedSimClustersTripParam` | Follow graph |
| TwHIN ANN | `EnableTwHINParam` | Graph embeddings |
| Two-Tower ANN | `EnableTwoTowerParam` | Neural embeddings |
| User Video Graph | (always on) | Video engagement |
| Consumer WALS | `EnableSourceParam` | Random walk embeddings |
| Customized Retrieval | (model-specific) | Various |

Per-source candidate limits controlled by `MaxCandidateNumPerSourceKeyParam`.

### 6.3 Similarity Engines (35 Total)

**Key directory**: `cr-mixer/server/src/main/scala/com/twitter/cr_mixer/similarity_engine/`

**Embedding-based**:
- `SimClustersANNSimilarityEngine` -- primary ANN via SimClusters
- `ConsumerEmbeddingBasedTwHINSimilarityEngine` -- TwHIN embeddings
- `ConsumerEmbeddingBasedTwoTowerSimilarityEngine` -- two-tower neural
- `ConsumerEmbeddingBasedTripSimilarityEngine` -- TRIP model
- `TwhinCollabFilterSimilarityEngine` -- collaborative filtering

**Graph-based**:
- `ProducerBasedUserTweetGraphSimilarityEngine` -- user-tweet graph
- `ConsumersBasedUserVideoGraphSimilarityEngine` -- video graph
- `UserTweetEntityGraphSimilarityEngine` -- UTEG/GraphJet
- `ProducerBasedUserAdGraphSimilarityEngine` -- ad graph

**Search-based**:
- `EarlybirdSimilarityEngine` -- real-time search wrapper
- `EarlybirdModelBasedSimilarityEngine` -- ML-enhanced search
- `EarlybirdRecencyBasedSimilarityEngine` -- time-decay search

**Specialized**:
- `SkitTopicTweetSimilarityEngine` -- topic classification
- `DiffusionBasedSimilarityEngine` -- viral spread prediction
- `ConsumerBasedWalsSimilarityEngine` -- WALS random walk

### 6.4 Blending Strategy

**Key file**: `cr-mixer/server/src/main/scala/com/twitter/cr_mixer/blender/SwitchBlender.scala`

Four blending strategies:

| Strategy | Method |
|----------|--------|
| `RoundRobin` (default) | Takes 1 candidate from each source in sequence |
| `SourceTypeBackFill` | Backfills from secondary sources when primary is thin |
| `SourceSignalSorting` | Ranks by source signal recency (newer = higher) |
| `ContentSignalBlending` | Content-based signal mixing |

**Ordering strategies**:
- `TimestampOrder`: Newer source signals rank higher. Consumer-based candidates default to timestamp 0 (lowest priority).
- `RandomOrder`: Random shuffle via `scala.util.Random.nextDouble()`

```scala
// From InterleaveBlender.scala
// Round-robin: "takes 1 candidate from each Seq in sequence,
// until we run out of candidates"
InterleaveUtil.interleave(candidateSequences)
```

### 6.5 Pre-Rank Filters

**Key file**: `cr-mixer/server/src/main/scala/com/twitter/cr_mixer/filter/PreRankFilterRunner.scala`

Applied sequentially via fold:

```scala
filters = Seq(
    TweetAgeFilter,          // remove tweets beyond max age
    ImpressedTweetlistFilter, // remove already-seen tweets
    VideoTweetFilter,         // video content type filter
    ReplyFilter               // filter tweet replies
)

// Additional filtering in CrCandidateGenerator:
// - Block/muted user removal via filterSourceInfo()
// - Negative signal extraction (partitions into positive/negative)
```

### 6.6 Ranking Parameters

```scala
// From CrCandidateGenerator.scala
RankerParams.MaxCandidatesToRank        // caps volume before ranking
RankerParams.EnableBlueVerifiedTopK     // prioritizes verified authors
RecentNegativeSignalParams.EnableSourceParam  // negative signal filtering
```

---

## 7. TwHIN

**Code path (training)**: `the-algorithm-ml/projects/twhin/`
**Code path (serving)**: `cr-mixer/server/src/main/scala/com/twitter/cr_mixer/similarity_engine/ConsumerEmbeddingBasedTwHINSimilarityEngine.scala`

**Paper**: [TwHIN: Embedding the Twitter Heterogeneous Information Network for Personalized Recommendation](https://arxiv.org/abs/2202.05387) (KDD 2022)

### 7.1 Graph Scale

| Metric | Value |
|--------|-------|
| Nodes | **>1 billion** (10^9) |
| Edges | **>100 billion** (10^11) |
| Entity types | 4: User, Tweet, Advertiser, Ad |
| Relation types | 7 |

### 7.2 Relation Types

| Relation | Left Entity | Right Entity |
|----------|------------|--------------|
| Follows | User | User |
| Authors | User | Tweet |
| Favorites | User | Tweet |
| Replies | User | Tweet |
| Retweets | User | Tweet |
| Promotes | Advertiser | Ad |
| Clicks | User | Ad |

### 7.3 TransE Architecture

**Key file**: `the-algorithm-ml/projects/twhin/models/models.py`

TwHIN uses a TransE-style scoring function:

```python
# From models.py

# Translation operation:
translated = x[:, 1, :] + trans_embs  # target + relation translation

# Positive score (dot product of source and translated target):
pos_score = (x[:, 0, :] * translated).sum(-1)

# Embedding dimensions: B x 2 x D
# where B = batch size, D = embedding dimension
# "2B x T x D" reduced to "2B x D" via summation across tables
```

### 7.4 Negative Sampling

```python
# In-batch negative sampling:
# 1. Group embeddings by relation type
# 2. Randomly permute to generate negatives
# 3. Compute negative scores via matrix multiplication
# 4. Balance: neg_weight = num_positives / num_negatives

loss = weighted_combination(positive_loss, negative_loss)
```

### 7.5 Training Configuration

| Parameter | Value |
|-----------|-------|
| Optimizer | Adagrad |
| Operator | Translation (TransE) |
| Framework | PyTorch + TorchRec (distributed) |
| Training | `apply_optimizer_in_backward` for per-relation optimizers |

**Validation** (from `config.py`):
```python
# All embedding tables must have matching dimensions
# All embedding tables must have matching data types
# All relation lhs/rhs must reference existing table names
```

### 7.6 Serving via HNSW ANN

```scala
// From ConsumerEmbeddingBasedTwHINSimilarityEngine.scala
// Constructs HnswANNEngineQuery (Hierarchical Navigable Small World)
// Model ID configured via ConsumerEmbeddingBasedTwHINParams.ModelIdParam
```

### 7.7 Production Impact

From the KDD 2022 paper:
- **Ads ranking**: 2.38 RCE gain, **10.3% cost-per-conversion reduction**
- Improvements shown for Who-To-Follow, search ranking, and content safety
- Online A/B tests confirmed offline gains

---

## 8. Impact on Small Accounts (~100 Followers)

Understanding how each component treats a small account with approximately 100 followers.

### 8.1 SimClusters: Mostly Invisible

**KnownFor eligibility**: Requires `minActiveFollowers = 400`. With 100 followers, **you are not in the KnownFor matrix**. Your account is not a "producer" in SimClusters terminology.

**Implication**: Your tweets can still get SimClusters embeddings through favorites (the real-time Heron job adds favoriter InterestedIn vectors), but you are not a seed in any community. Discovery depends entirely on someone with a strong InterestedIn vector finding and favoriting your tweet through other means first.

**InterestedIn**: As a consumer, your InterestedIn is computed normally based on who you follow. You will receive out-of-network recommendations. But your content is unlikely to appear in others' feeds via SimClusters unless it achieves initial engagement momentum.

### 8.2 Real Graph: Weak Edges

With 100 followers, you have at most 100 incoming edges. The Real Graph decay (ONE_MINUS_ALPHA = 0.955) means edges without recent interaction fade quickly. If followers do not actively engage with your tweets, your Real Graph edges become near-zero within weeks.

**Result**: You are unlikely to appear as a seed user in anyone's candidate generation unless you have very recent, direct engagement with that person.

### 8.3 EarlyBird: The TweepCred Gate

**The single biggest obstacle**: TweepCred PageRank penalizes accounts with low followers and high following ratios.

```
if (tweepcred < 65):
    max_tweets_considered = 3
```

With 100 followers, your TweepCred is likely well below 65. This means **only your 3 most recent tweets are even evaluated** by the algorithm. If you tweet 5 times a day, tweets 4 and 5 are invisible to ranking regardless of quality.

**Age decay** compounds this: with a 360-minute half-life, even your 3 eligible tweets lose 50% relevance every 6 hours.

**No media boost leverage**: The 2x media boost helps, but applied to a low base score (low reputation, low engagement), it may not push candidates above the threshold.

### 8.4 GraphJet/UTEG: Cold Start Problem

UTEG relies on engagement edges in the bipartite graph. A tweet from a 100-follower account that receives 0-2 favorites creates minimal graph signal. The SALSA walk is unlikely to reach your tweets because:

1. Few users engaged with your tweet (low authority score)
2. Those few users may not be in anyone's "circle of trust"
3. The `MinUserSocialProof = 1` threshold is low, but your tweet needs to be discovered in the walk first

**However**: If even one well-connected user (high PageRank) favorites your tweet, it enters the UTEG graph with meaningful signal. UTEG is the most accessible path for small accounts to reach out-of-network users.

### 8.5 TwHIN: Representation Exists

TwHIN embeds all users (>1B nodes), so your account has an embedding. But with only 100 follow edges and minimal engagement edges, your embedding is sparse and low-confidence. The ANN search is less likely to surface your content because the dot-product similarity with other users' embeddings will be low.

### 8.6 CrMixer: No Explicit Penalty, But Structural Disadvantage

CrMixer does not penalize small accounts directly. But since every upstream source (SimClusters, UTEG, TwHIN) produces weaker signals for small accounts, fewer candidates from small accounts survive to the blending and ranking stages.

### 8.7 Summary: The Small Account Bottleneck

```
Component          | Barrier for ~100 followers
-------------------+--------------------------------------------
SimClusters        | Not in KnownFor (need 400 followers)
                   | Tweet embedding requires favorites first
Real Graph         | Weak edges decay in ~7 days without engagement
EarlyBird          | TweepCred < 65 -> only 3 tweets evaluated
                   | 360-min half-life punishes infrequent posters
GraphJet/UTEG      | Cold start -- need engagement to enter graph
TwHIN              | Sparse embedding, low similarity scores
CrMixer            | No direct penalty but all sources are weak

Best strategy:     | Get engagement from high-TweepCred users
                   | Use images/video (2x EarlyBird boost)
                   | Post consistently (3-tweet cap per cycle)
                   | Reply to larger accounts (creates UTEG edges)
```

---

## 9. Appendix: Key File Paths

### SimClusters v2

```
src/scala/com/twitter/simclusters_v2/
  scalding/
    update_known_for/
      UpdateKnownFor20M145K2020.scala       # KnownFor batch job (20M producers, 145K clusters)
      UpdateKnownForSBFRunner.scala          # SBF + Metropolis-Hastings clustering
    InterestedInFromKnownFor.scala           # InterestedIn matrix computation
    InterestedInFromKnownForLite.scala       # Lightweight variant
    UserUserGraph.scala                      # User similarity graph
    UserUserFavGraph.scala                   # Favorite-based user graph
    EigenVectorsForSparseSymmetric.scala     # Eigenvalue decomposition
  summingbird/storm/
    TweetJobRunner.scala                     # Storm topology configuration
    TweetJob.scala                           # Real-time tweet embedding updates
  candidate_source/
    SimClustersANNCandidateSource.scala       # ANN candidate retrieval
    ClusterRanker.scala                      # Cluster-based ranking
```

### Real Graph

```
src/scala/com/twitter/interaction_graph/
  scio/agg_all/
    InteractionGraphAggregationConfig.scala   # ALPHA=1.0, ONE_MINUS_ALPHA=0.955
    InteractionGraphAggregationJob.scala      # Daily aggregation pipeline
    InteractionGraphAggregationTransform.scala # Post-aggregation top-K selection
  bqe/                                        # Gradient boosting in BigQuery
```

### TweepCred

```
src/scala/com/twitter/graph/batch/job/tweepcred/
  PreparePageRankData.scala                   # Graph construction
  UpdatePageRank.scala                        # Iterative PageRank
  README                                      # Algorithm documentation
```

### EarlyBird

```
src/java/com/twitter/search/earlybird/
  search/relevance/scoring/
    ScoringFunction.java                     # Abstract scoring base
    LinearScoringFunction.java               # Linear combination scorer
    LinearScoringData.java                   # All feature definitions + constants
    ModelBasedScoringFunction.java           # ML model scorer
    TensorflowBasedScoringFunction.java      # Deep learning scorer
  document/                                   # Tweet indexing
  index/                                      # Inverted index, facets
  queryparser/                                # Query parsing
  querycache/                                 # Result caching

src/python/twitter/deepbird/projects/timelines/scripts/models/earlybird/
  README.md                                   # Light ranker documentation
  train.py                                    # Model training script
```

### CrMixer

```
cr-mixer/server/src/main/scala/com/twitter/cr_mixer/
  candidate_generation/
    CrCandidateGenerator.scala                # Main pipeline orchestrator
    CandidateSourcesRouter.scala              # Routes to SimClusters/TwHIN/UTEG
    UtegTweetCandidateGenerator.scala          # UTEG-specific generator
    SimClustersInterestedInCandidateGeneration.scala
    FrsTweetCandidateGenerator.scala           # Friend recommendations
    TopicTweetCandidateGenerator.scala         # Topic-based
    RelatedTweetCandidateGenerator.scala       # Related tweets
  similarity_engine/
    SimClustersANNSimilarityEngine.scala       # SimClusters ANN
    ConsumerEmbeddingBasedTwHINSimilarityEngine.scala  # TwHIN
    UserTweetEntityGraphSimilarityEngine.scala  # UTEG
    EarlybirdSimilarityEngine.scala            # EarlyBird wrapper
    EarlybirdRecencyBasedSimilarityEngine.scala # Recency ranking
    TwhinCollabFilterSimilarityEngine.scala     # Collaborative filtering
    DiffusionBasedSimilarityEngine.scala        # Viral prediction
    ConsumerBasedWalsSimilarityEngine.scala     # WALS embeddings
  blender/
    SwitchBlender.scala                        # Strategy router
    InterleaveBlender.scala                    # Round-robin interleaving
    SourceTypeBackFillBlender.scala            # Backfill strategy
    ContentSignalBlender.scala                 # Content-based mixing
  filter/
    PreRankFilterRunner.scala                  # Age, impression, video, reply filters
    PostRankFilterRunner.scala                 # Post-rank filtering
  source_signal/
    SourceInfoRouter.scala                     # USS + FRS signal collection
```

### TwHIN

```
the-algorithm-ml/projects/twhin/
  config.py                                   # Top-level configuration
  models/
    config.py                                 # TwhinModelConfig, TwhinEmbeddingsConfig
    models.py                                 # TransE model, negative sampling, loss
  data/                                       # Data loading
  optimizer.py                                # Per-relation optimizers
  run.py                                      # Training entry point
  metrics.py                                  # Evaluation metrics
```

### GraphJet

```
External repository: github.com/twitter/GraphJet
Java library -- in-memory bipartite graph + SALSA

In main repo:
cr-mixer/.../UserTweetEntityGraphSimilarityEngine.scala  # Integration point
cr-mixer/.../UtegTweetCandidateGenerator.scala            # Pipeline wrapper
```

---

## References

- [Twitter Engineering Blog: Twitter's Recommendation Algorithm](https://blog.x.com/engineering/en_us/topics/open-source/2023/twitter-recommendation-algorithm)
- [twitter/the-algorithm on GitHub](https://github.com/twitter/the-algorithm)
- [twitter/the-algorithm-ml on GitHub](https://github.com/twitter/the-algorithm-ml)
- [twitter/GraphJet on GitHub](https://github.com/twitter/GraphJet)
- [TwHIN KDD 2022 Paper (arXiv)](https://arxiv.org/abs/2202.05387)
- [GraphJet: Real-Time Content Recommendations at Twitter (VLDB)](https://dl.acm.org/doi/abs/10.14778/3007263.3007267)
- [EarlyBird: Real-Time Search at Twitter](https://notes.stephenholiday.com/Earlybird.pdf)
- [Steven Tey: How the Twitter Algorithm Works](https://steventey.com/blog/twitter-algorithm)
- [Sumit's Diary: Twitter's For You Recommendation Algorithm](https://blog.reachsumit.com/posts/2023/04/the-twitter-ml-algo/)
- [Dive into Twitter's Recommendation System II - GraphJet](https://happystrongcoder.substack.com/p/dive-into-twitters-recommendation-7cd)
- [Igor Brigadir: Awesome Twitter Algo](https://github.com/igorbrigadir/awesome-twitter-algo)
