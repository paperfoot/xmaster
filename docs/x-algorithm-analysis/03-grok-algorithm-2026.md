# X "For You" Feed Algorithm -- January 2026 (Grok-Powered)

Source code: [github.com/xai-org/x-algorithm](https://github.com/xai-org/x-algorithm) (Apache 2.0)
Released: January 20, 2026 | Languages: Rust 62.9%, Python 37.1% | 16,000+ GitHub stars in first week

---

## 1. System Architecture

The 2026 algorithm is a clean-room rewrite of X's recommendation stack. Four modules compose the "For You" feed:

```
                    +------------------+
                    |   Home Mixer     |   gRPC orchestrator (Rust)
                    |  ScoredPosts     |
                    |    Service       |
                    +--------+---------+
                             |
              +--------------+--------------+
              |                             |
    +---------v--------+         +----------v---------+
    |     Thunder      |         |      Phoenix       |
    |  (In-Network)    |         |  (Out-of-Network)  |
    |  Kafka -> memory |         |  Two-tower + Grok  |
    |  sub-ms lookups  |         |  transformer       |
    +------------------+         +--------------------+
              |                             |
              +-------------+---------------+
                            |
                   +--------v--------+
                   | Candidate       |
                   | Pipeline        |
                   | (trait-based    |
                   |  framework)     |
                   +-----------------+
```

### 1.1 Home Mixer (`home-mixer/`)

The gRPC entry point. `HomeMixerServer` implements `ScoredPostsService` with a single RPC:

```rust
// home-mixer/server.rs
async fn get_scored_posts(
    &self,
    request: Request<pb::ScoredPostsQuery>,
) -> Result<Response<ScoredPostsResponse>, Status>
```

Request payload includes `viewer_id`, `client_app_id`, `country_code`, `language_code`, `seen_ids`, `served_ids`, `in_network_only`, `is_bottom_request`, and `bloom_filter_entries`. The server delegates to `PhoenixCandidatePipeline::execute(query)`, then maps resulting `PostCandidate` structs into protobuf `ScoredPost` responses.

Key response fields per post: `tweet_id`, `author_id`, `retweeted_tweet_id`, `score`, `in_network`, `served_type`, `prediction_request_id`, `ancestors`, `screen_names`, `visibility_reason`.

### 1.2 Thunder (`thunder/`)

In-memory post store consuming Kafka events in real time.

```
thunder/
  kafka/           -- Kafka consumer/partitioner
  posts/           -- Per-user post store (originals, replies/reposts, video)
  deserializer.rs  -- Protobuf -> Rust struct
  kafka_utils.rs
  thunder_service.rs  -- gRPC service: get_in_network_posts
  main.rs          -- Entry point: PostStore + StratoClient + Kafka init
```

**ThunderServiceImpl** handles `get_in_network_posts`:
1. Semaphore-based concurrency control -- rejects with `RESOURCE_EXHAUSTED` when at capacity
2. Fetches the viewer's following list from Strato (capped at `MAX_INPUT_LIST_SIZE`)
3. Retrieves posts from in-memory `PostStore` via `spawn_blocking` (no async runtime blocking)
4. Sorts by recency, limits to `max_results`
5. Reports detailed metrics: freshness, time range, reply ratio, unique authors, posts per author

**Lifecycle** (from `main.rs`):
- PostStore initialized with configurable retention period (seconds)
- Kafka consumer creates channel for post events
- Auto-trim task runs every 2 minutes, removing expired posts
- HTTP + gRPC servers on separate ports

### 1.3 Phoenix (`phoenix/`)

The ML brain -- a JAX-based two-stage recommendation system ported from Grok-1.

```
phoenix/
  grok.py                     -- Transformer architecture (from Grok-1)
  recsys_retrieval_model.py   -- Two-tower retrieval
  recsys_model.py             -- Ranking model
  run_retrieval.py            -- Retrieval runner
  run_ranker.py               -- Ranking runner
  runners.py                  -- Shared utilities
  test_recsys_model.py
  test_recsys_retrieval_model.py
  README.md
```

Details in Section 2 below.

### 1.4 Candidate Pipeline (`candidate-pipeline/`)

A reusable, trait-based framework with composable stages:

```rust
// candidate-pipeline/candidate_pipeline.rs
enum PipelineStage {
    QueryHydrator,
    Source,
    Hydrator,
    PostSelectionHydrator,
    Filter,
    PostSelectionFilter,
    Scorer,
}
```

**Core traits** (all async, generic over `Q: Query` and `C: Candidate`):

| Trait | Method | Purpose |
|-------|--------|---------|
| `Source<Q, C>` | `get_candidates(&self, query) -> Vec<C>` | Fetch candidates |
| `Hydrator<Q, C>` | (enrichment) | Augment candidate metadata |
| `Filter<Q, C>` | `filter(&self, query, candidates) -> FilterResult<C>` | Partition into kept/removed |
| `Scorer<Q, C>` | `score(&self, query, candidates) -> Vec<C>` | Assign scores (same order, no drops) |
| `Selector<Q, C>` | (selection) | Top-K extraction |
| `SideEffect<Q, C>` | (logging/metrics) | Post-pipeline effects |

**Execution flow** inside `CandidatePipeline::execute()`:
1. Hydrate query (parallel `join_all`)
2. Fetch candidates from all sources (parallel `join_all`)
3. Hydrate candidates (parallel)
4. Sequential filtering -- each filter partitions kept/removed
5. Sequential scoring
6. Selection (top-K)
7. Post-selection hydration and filtering
8. Side effects (parallel)

Error resilience: failures are logged but do not halt the pipeline. Length validation ensures hydrators and scorers preserve candidate count and order.

---

## 2. Phoenix Ranking Engine

### 2.1 Two-Tower Retrieval (`recsys_retrieval_model.py`)

Reduces millions of corpus posts to thousands of candidates.

**User Tower**:
- Combines user hash embeddings + engagement history via `block_user_reduce()` and `block_history_reduce()`
- Processes through the Phoenix transformer (shared architecture with ranking model)
- Outputs L2-normalized user representation `[B, D]`

**Candidate Tower** (`CandidateTower`):
- MLP that projects post + author embeddings to a shared embedding space
- Reshapes concatenated embeddings, applies two projection layers with SiLU activation
- Returns L2-normalized candidate representations `[N, D]`

**Retrieval**:
- `_retrieve_top_k()`: dot-product similarity between user and candidate vectors
- Efficient approximate nearest neighbor (ANN) search
- Supports optional corpus masking during retrieval
- Returns `RetrievalOutput`: user representation + top-K indices and scores

### 2.2 Grok Transformer (`grok.py`)

The transformer architecture is ported directly from [Grok-1](https://github.com/xai-org/grok-1), adapted for recommendation tasks.

**Core components**:

| Class | Function |
|-------|----------|
| `Transformer` | Multi-layer transformer stack |
| `DecoderLayer` | Attention + feed-forward per layer |
| `MultiHeadAttention` | Grouped query attention with rotary embeddings (RoPE) |
| `RotaryEmbedding` | Per [arXiv:2104.09864](https://arxiv.org/abs/2104.09864) |
| `RMSNorm` | Root mean square normalization |
| `Linear` | Custom linear layer with fp32 weight storage |
| `MHABlock` / `DenseBlock` | Modular attention and dense sub-layers |

**Critical function -- `make_recsys_attn_mask()`**:

Creates the candidate isolation mask:
- User + history sections: full bidirectional attention
- Candidates can attend to user + history
- Candidates can self-attend (diagonal only)
- **Cross-candidate attention is blocked**

This ensures scoring is independent of batch composition, enabling consistent and cacheable per-post scores.

Implementation uses JAX + Haiku with `bfloat16` precision by default.

### 2.3 Ranking Model (`recsys_model.py`)

**PhoenixModel** processes inputs through:

1. **Embedding lookup**: Hash-based embeddings using multiple hash functions (memory-efficient, trades precision for scale)
   - Actions encoded as multi-hot vectors
   - Product surfaces via categorical embeddings
2. **Input assembly**: Combines user, history (128 sequence length), and candidate (32 sequence length) embeddings via:
   - `block_user_reduce()`: multiple user hash embeddings -> single user representation via projection matrices
   - `block_history_reduce()`: concatenates post + author + action + product surface embeddings, then projects to embedding dimension
   - `block_candidate_reduce()`: concatenates post + author + product surface embeddings, then projects
3. **Transformer processing**: With the candidate isolation attention mask
4. **Layer normalization + unembedding**: Generates ranking logits

**Input structure**:
- User embeddings: `[B, 1]`
- History embeddings: `[B, S=128, D]` (posts, authors, actions, product surface)
- Candidate embeddings: `[B, C=32, D]` (posts, authors, product surface)

**Output**: `[B, num_candidates, num_actions]` -- logits for each candidate across all engagement types.

### 2.4 The 15 Output Probabilities

The model simultaneously predicts the probability of these user actions:

| # | Action | Signal Type |
|---|--------|-------------|
| 1 | `P(favorite)` | Positive |
| 2 | `P(reply)` | Positive |
| 3 | `P(repost)` | Positive |
| 4 | `P(quote)` | Positive |
| 5 | `P(click)` | Positive |
| 6 | `P(profile_click)` | Positive |
| 7 | `P(video_view)` | Positive |
| 8 | `P(photo_expand)` | Positive |
| 9 | `P(share)` | Positive |
| 10 | `P(dwell)` | Positive |
| 11 | `P(follow_author)` | Positive |
| 12 | `P(not_interested)` | **Negative** |
| 13 | `P(block_author)` | **Negative** |
| 14 | `P(mute_author)` | **Negative** |
| 15 | `P(report)` | **Negative** |

The PhoenixScorer (`home-mixer/scorers/phoenix_scorer.rs`) extracts these by:
1. Calling `self.phoenix_client.predict(user_id, sequence, tweet_infos)` via async gRPC
2. Converting log probabilities via `(*log_prob as f64).exp()`
3. Mapping to specific action types in a `PhoenixScores` struct (19 fields total, including continuous metrics like dwell time)
4. Handling retweets by using original tweet IDs

---

## 3. WeightedScorer (`home-mixer/scorers/weighted_scorer.rs`)

The `WeightedScorer` struct combines Phoenix predictions into a single composite score.

### 3.1 Scoring Formula

```
Final Score = sum( weight_i * P(action_i) )    for i in 1..19
```

The `apply()` method: `score.unwrap_or(0.0) * weight`

Approximately 19 engagement metrics are combined, loaded from `crate::params`:
- `FAVORITE_WEIGHT`
- `REPLY_WEIGHT`
- `RETWEET_WEIGHT`
- `DWELL_WEIGHT`
- `VQV_WEIGHT` (video quality view)
- Plus weights for quote, click, profile_click, video_view, photo_expand, share, follow_author
- Negative weights for not_interested, block_author, mute_author, report

**Video-specific logic**: Video quality view scores receive conditional weighting -- a duration eligibility check (`ms > p::MIN_VIDEO_DURATION_MS`) determines whether VQV weight applies.

### 3.2 Score Normalization

The `offset_score()` method handles three scenarios:
1. **Zero weight sum**: Returns raw score
2. **Negative combined score**: Applies offset using `NEGATIVE_WEIGHTS_SUM` and `NEGATIVE_SCORES_OFFSET`
3. **Positive score**: Normalizes using `WEIGHTS_SUM`

### 3.3 Engagement Weight Hierarchy (from external analyses)

While the actual weight constants in `crate::params` are not published in the repo, independent analysis of the scoring behavior reveals this approximate hierarchy (normalized to Like = 1x):

| Signal | Approximate Multiplier |
|--------|----------------------|
| Author-replied-to-user reply | ~75x |
| Profile click + engagement | ~24x |
| Full video view | ~22x |
| Retweet | ~20x |
| Long dwell / deep read | ~14x |
| Reply | ~13.5x |
| Conversation click + engagement | ~11x |
| Bookmark | ~10x |
| Like | 1x |

**Negative signals** (asymmetric, much heavier than positive):

| Signal | Impact |
|--------|--------|
| Report | Severe negative (~-1,500 reach units) |
| Block / Mute | Heavy negative (~-500 reach units) |
| "Not interested" / "See less" | Moderate negative (~-50 reach units) |

The asymmetry is deliberate: negative signals are weighted 6-12x heavier than positive signals, reflecting retention-focused optimization.

### 3.4 All Four Scorers

The scoring pipeline (`home-mixer/scorers/mod.rs`) declares four scorers applied sequentially:

| Scorer | Purpose |
|--------|---------|
| `phoenix_scorer` | Invokes Phoenix transformer, extracts 19 action probabilities |
| `weighted_scorer` | Combines probabilities via weighted sum formula |
| `author_diversity_scorer` | Exponential decay for repeated authors: `(1-floor) * decay^position + floor` |
| `oon_scorer` | Multiplies out-of-network posts by `OON_WEIGHT_FACTOR` |

**AuthorDiversityScorer** details:
- Tracks author frequency via HashMap
- Sorts candidates by weighted score (descending)
- Applies positional decay: each successive post from the same author gets a progressively lower multiplier
- Configurable `decay_factor` and `floor` from params

**OONScorer** details:
- In-network posts retain original scores
- Out-of-network posts: `base_score * p::OON_WEIGHT_FACTOR`
- Ensures in-network content is prioritized while still surfacing discoveries

---

## 4. Pre-Scoring Filters

The `home-mixer/filters/` module declares 12 filter submodules. The pipeline executes these sequentially, with each filter partitioning candidates into kept and removed sets.

### 4.1 Filter Inventory

| # | Filter | Purpose |
|---|--------|---------|
| 1 | `DropDuplicatesFilter` | Eliminates redundant candidate posts |
| 2 | `AgeFilter` | Removes posts older than `max_age` (configurable Duration; uses Snowflake ID timestamp extraction via `snowflake::duration_since_creation_opt()`) |
| 3 | `SelfTweetFilter` | Hides the viewer's own posts from their feed |
| 4 | `PreviouslySeenPostsFilter` | Excludes posts the viewer has already seen |
| 5 | `PreviouslyServedPostsFilter` | Excludes posts already served in a prior request |
| 6 | `MutedKeywordFilter` | Tokenizes post text and matches against user's muted keyword list via `TweetTokenizer` + `UserMutes` matcher |
| 7 | `AuthorSocialgraphFilter` | Filters based on author social graph relationships (blocked/muted authors) |
| 8 | `RetweetDeduplicationFilter` | Consolidates duplicate retweets of the same original post |
| 9 | `CoreDataHydrationFilter` | Removes candidates where essential data hydration failed |
| 10 | `IneligibleSubscriptionFilter` | Excludes subscription-gated content the viewer cannot access |
| 11 | `DedupConversationFilter` | Removes duplicate entries within conversation threads |
| 12 | `VFFilter` | Vendor-specific / visibility framework filtering |

### 4.2 Post-Selection Filters

After scoring and top-K selection, additional validation occurs:
- Final visibility checks
- Conversation thread deduplication
- Deleted/spam content removal

### 4.3 Filter Trait Design

```rust
// candidate-pipeline/filter.rs
#[async_trait]
pub trait Filter<Q, C>: Any + Send + Sync {
    fn enable(&self, _query: &Q) -> bool { true }
    async fn filter(&self, query: &Q, candidates: Vec<C>) -> FilterResult<C>;
    fn name(&self) -> &'static str;
}

struct FilterResult<C> {
    kept: Vec<C>,
    removed: Vec<C>,
}
```

---

## 5. Candidate Sources

Two sources feed the pipeline (`home-mixer/sources/mod.rs`):

| Source | Backend | Content Type |
|--------|---------|-------------|
| `thunder_source` | Thunder gRPC | In-network posts from followed accounts |
| `phoenix_source` | Phoenix retrieval | Out-of-network posts via two-tower similarity |

Approximate split: ~1,500 candidates sourced from ~500M daily posts (roughly 50% in-network, 50% out-of-network).

### Selection

A single selector is defined (`home-mixer/selectors/mod.rs`):

**`TopKScoreSelector`** -- sorts by final composite score, returns top K posts for the feed response.

---

## 6. Full Pipeline Flow

```
Request (viewer_id, seen_ids, served_ids, ...)
    |
    v
[Query Hydration] -- fetch user engagement history, metadata
    |
    v
[Candidate Sourcing] -- parallel:
    |-- Thunder: in-network posts (sub-ms from memory)
    |-- Phoenix Retrieval: out-of-network via two-tower ANN search
    |
    v
[Candidate Hydration] -- enrich with post data, author info, media metadata
    |
    v
[Pre-Scoring Filters] -- sequential, 12 filters:
    DropDuplicates -> Age -> SelfTweet -> PreviouslySeen ->
    PreviouslyServed -> MutedKeyword -> AuthorSocialgraph ->
    RetweetDedup -> CoreDataHydration -> IneligibleSubscription ->
    DedupConversation -> VF
    |
    v
[Scoring] -- sequential, 4 scorers:
    Phoenix (15 probabilities) -> Weighted (composite score) ->
    AuthorDiversity (decay) -> OON (weight factor)
    |
    v
[Selection] -- TopKScoreSelector
    |
    v
[Post-Selection Hydration] -- additional data enrichment
    |
    v
[Post-Selection Filtering] -- visibility validation, thread dedup
    |
    v
[Side Effects] -- logging, metrics, analytics (parallel)
    |
    v
Response (Vec<ScoredPost> with score, in_network, ancestors, ...)
```

Target latency: ~200ms end-to-end across hundreds of millions of concurrent requests.

---

## 7. Key Differences from the 2023 Algorithm

The 2023 open-source release ([github.com/twitter/the-algorithm](https://github.com/twitter/the-algorithm)) was a Java/Scala monolith with Python ML. The 2026 rewrite is fundamentally different:

### 7.1 Architecture

| Aspect | 2023 | 2026 |
|--------|------|------|
| **Language** | Java/Scala + Python | Rust + Python (JAX) |
| **ML Model** | 48M-parameter neural network (MaskNet) | Grok-1 transformer adapted for recommendations |
| **Feature Engineering** | Thousands of hand-crafted features | Zero hand-engineered features -- transformer learns from raw engagement sequences |
| **Out-of-network discovery** | SimClusters (145K topic clusters via matrix factorization) | Phoenix two-tower embeddings with dot-product ANN retrieval |
| **Scoring** | MaskNet with feature interactions | Grok transformer with candidate isolation masking |
| **Ranking signal count** | ~12 engagement types | 15 engagement types (added video_view, photo_expand, share, dwell) |
| **Code architecture** | Monolithic service mesh | Four modular components with trait-based pipeline |

### 7.2 SimClusters -> Phoenix Two-Tower Embeddings

**2023 (SimClusters)**: Matrix factorization-based community detection grouping users into ~145,000 topic clusters. Out-of-network posts were surfaced by matching post clusters to user clusters via cosine similarity. Dense, precomputed, updated periodically.

**2026 (Phoenix Two-Tower)**: Continuous embedding model with separate user and candidate towers. User tower encodes real-time engagement history through the transformer. Candidate tower projects post+author features via MLP. Retrieval via dot-product similarity with ANN search. Embeddings update with every interaction -- no batch recomputation.

### 7.3 MaskNet -> Grok Transformer

**2023 (MaskNet)**: A specialized neural architecture using feature masking to model implicit feature interactions. Required extensive hand-engineered feature extraction as input.

**2026 (Grok Transformer)**: Adapted from xAI's Grok-1 language model. Uses the same attention mechanism, rotary embeddings, and layer structure. Key adaptation: `make_recsys_attn_mask()` implements candidate isolation -- posts cannot attend to each other during inference, only to user context and history. This is the critical architectural insight that enables consistent, cacheable, batch-independent scoring.

### 7.4 Sentiment & Tone Analysis

**2023**: Complex "toxicity" rules and content moderation heuristics.

**2026**: Grok-powered sentiment analysis operates as part of the transformer's learned representations. Positive and constructive messaging gets wider distribution; negative and combative tones lead to reduced visibility. This is an emergent property of the engagement prediction model rather than a hand-coded rule -- the model learns that combative content correlates with negative actions (block, mute, report) and naturally downranks it through the weighted scoring formula where negative signals carry asymmetrically heavy weights.

### 7.5 Other Notable Changes

- **Hash-based embeddings** replace traditional lookup tables, reducing memory footprint at scale
- **Author diversity scoring** is now an explicit pipeline stage rather than a post-hoc heuristic
- **Candidate isolation masking** is a novel contribution enabling per-post score caching
- **Continuous dwell time** is now a first-class prediction target alongside binary actions
- **Video quality view** has conditional weighting based on minimum duration threshold

---

## 8. What Is NOT in the Repo

The open-source release is intentionally incomplete. Missing elements:

- **Actual weight constants** in `crate::params` -- numerical values for `FAVORITE_WEIGHT`, `REPLY_WEIGHT`, etc. are not published
- **Pre-trained model weights** -- the Phoenix transformer checkpoint is absent
- **Training data** -- no datasets, sampling methodology, or training pipelines
- **Advertising integration** -- the ad ranking system is excluded
- **TweepCred / reputation scores** -- the hidden account quality scoring system (range -128 to +100) is not in the repo
- **Scaling optimizations** -- production-specific performance tuning omitted
- **Deployment configuration** -- infrastructure specs, cluster topology, GPU allocation
- **A/B testing framework** -- experimentation system not included

X states the repository represents "the model used internally with the exception of specific scaling optimizations" and commits to quarterly updates with developer notes.

---

## 9. Implications for Content Strategy

Based on the scoring weights and pipeline design:

1. **Replies are king** -- author-engaged reply threads carry ~75x a like. Provoking genuine conversation is the highest-leverage action.
2. **Profile clicks signal deep interest** -- ~24x weight. Content that makes people want to see more of you is strongly rewarded.
3. **Video completion matters** -- ~22x for full views, with minimum duration gating. Short, watchable videos beat long ones that get abandoned.
4. **Negative signals are nuclear** -- a single report can outweigh hundreds of likes. Combative or spammy content gets severe penalties.
5. **Author diversity decay** -- posting 10 times won't give you 10x the reach. Each successive post from the same author in a feed batch gets exponentially attenuated.
6. **In-network posts are favored** -- the OON weight factor explicitly discounts out-of-network content. Your followers see you first.
7. **External links are penalized** -- keeping engagement on-platform is structurally incentivized.
8. **Niche consistency helps retrieval** -- the two-tower model builds a user embedding from engagement history. Consistent engagement within a topic strengthens the retrieval signal.

---

## Sources

- [xai-org/x-algorithm on GitHub](https://github.com/xai-org/x-algorithm)
- [X Engineering announcement](https://x.com/XEng/status/2013471689087086804)
- [How the Twitter/X Algorithm Works in 2026 -- PostEverywhere](https://posteverywhere.ai/blog/how-the-x-twitter-algorithm-works)
- [Breaking Down X's Open Source Algorithm -- Ajit Singh](https://singhajit.com/system-design/x-twitter-for-you-algorithm/)
- [The X Algorithm Explained -- Wallaroo Media](https://wallaroomedia.com/x-algorithm-explained/)
- [X open-sources algorithm powered by Grok AI -- EONMSK](https://www.eonmsk.com/2026/01/20/x-open-sources-algorithm-powered-by-grok-ai/)
- [X algorithm source code drops -- PPC Land](https://ppc.land/xs-algorithm-source-code-drops-what-it-reveals-about-the-platforms-feed-mechanics/)
- [X Open-Sources Grok-Powered Algorithm -- KAD](https://www.kad8.com/software/x-open-sources-its-recommendation-algorithm-built-on-grok-transformers/)
- [X Open-Sources Grok-Powered Algorithm -- Decrypt](https://decrypt.co/355108/elon-musks-x-open-sources-grok-powered-algorithm-driving-for-you-feed)
- [Phoenix Scoring, Embedding Models -- Bryant McGill](https://bryantmcgill.substack.com/p/phoenix-scoring-embedding-models)
- [New Rules for Viral Growth -- Slim Boulahouech / Medium](https://medium.com/@slim.boulahouech/they-just-open-sourced-the-x-algorithm-here-are-the-new-rules-for-viral-growth-aefbcfc84e76)
- [X open sources algorithm -- TechCrunch](https://techcrunch.com/2026/01/20/x-open-sources-its-algorithm-while-facing-a-transparency-fine-and-grok-controversies/)
- [X open sources algorithm -- VentureBeat](https://venturebeat.com/data/x-open-sources-its-algorithm-5-ways-businesses-can-benefit)
- [Elon Musk Open-Sources X Algorithm -- 36Kr](https://eu.36kr.com/en/p/3647512439918212)
