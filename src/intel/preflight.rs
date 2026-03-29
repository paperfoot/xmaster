use serde::Serialize;

/// Heuristic post quality analysis. Checks for common patterns that hurt or
/// help reach based on estimated 2026 X algorithm signals. This is NOT a direct
/// algorithm score — it's a quality lint.
///
/// Reference: xai-org/x-algorithm (January 2026, Grok-based transformer).
/// Exact weight constants are NOT published. Estimates below from code structure
/// + empirical data.
///
/// Top positive signals (estimated): follow_author (~30x), share_via_dm (~25x),
///   reply (~20x), share_via_copy_link (~20x), quote (~18x), profile_click (~12x)
/// Negative signals: report (~-369x), block (~-74x), mute (~-40x), not_interested (~-20x)
pub const ALGORITHM_SOURCE: &str = "xai-org/x-algorithm (January 2026, Grok-based)";

#[derive(Debug, Clone, Serialize)]
pub struct PreflightResult {
    pub text: String,
    pub score: u32,
    pub grade: String,
    pub issues: Vec<Issue>,
    pub suggestions: Vec<String>,
    pub features: FeatureVector,
    pub suggested_next_commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Critical => write!(f, "CRITICAL"),
            Severity::Warning => write!(f, "WARNING"),
            Severity::Info => write!(f, "INFO"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureVector {
    pub char_count: usize,
    pub word_count: usize,
    pub has_link: bool,
    pub link_position: Option<String>,
    pub has_media: bool,
    pub hashtag_count: usize,
    pub has_question: bool,
    pub has_numbers: bool,
    pub has_cta: bool,
    pub hook_strength: u32,
    pub line_count: usize,
    pub starts_with_i: bool,
    pub content_type_guess: String,
    /// Estimated dwell time in seconds (based on ~200 words/min reading speed)
    pub est_dwell_seconds: f64,
    /// Sentiment: positive, neutral, or negative (simple heuristic)
    pub sentiment: String,
}

/// Core pre-flight analysis. Evaluates tweet text and returns a scored result.
pub fn analyze(text: &str, goal: Option<&str>) -> PreflightResult {
    let trimmed = text.trim();
    let features = extract_features(trimmed);
    let mut issues = Vec::new();
    let mut score: i32 = 70;

    // --- Critical issues (score -= 30) ---
    if trimmed.is_empty() {
        issues.push(Issue {
            severity: Severity::Critical,
            code: "empty_content".into(),
            message: "Tweet is empty or whitespace-only".into(),
            fix: Some("Add tweet text".into()),
        });
        score -= 30;
    }

    // X Premium allows up to 25,000 characters. Standard accounts: 280.
    // We warn at 280 (standard visibility) but only mark critical at 25,000.
    if features.char_count > 25_000 {
        issues.push(Issue {
            severity: Severity::Critical,
            code: "over_limit".into(),
            message: format!(
                "Post is {} characters (X Premium limit: 25,000)",
                features.char_count
            ),
            fix: Some(format!(
                "Remove {} characters",
                features.char_count - 25_000
            )),
        });
        score -= 30;
    } else if features.char_count > 280 {
        issues.push(Issue {
            severity: Severity::Info,
            code: "long_post".into(),
            message: format!(
                "Post is {} characters (over 280; requires X Premium)",
                features.char_count
            ),
            fix: None,
        });
        // No score penalty — long posts are fine for Premium users
    }

    if features.has_link && features.link_position.as_deref() == Some("body") {
        issues.push(Issue {
            severity: Severity::Critical,
            code: "link_in_body".into(),
            message: "External link in tweet body kills reach — X suppresses linked tweets".into(),
            fix: Some("Move the link to a reply instead".into()),
        });
        score -= 30;
    }

    // --- Warning issues (score -= 15) ---
    let first_line = trimmed.lines().next().unwrap_or("");
    let weak_openers = ["I ", "So ", "Just ", "The "];
    if weak_openers.iter().any(|w| first_line.starts_with(w)) {
        issues.push(Issue {
            severity: Severity::Warning,
            code: "weak_hook".into(),
            message: format!(
                "Weak opening — \"{}...\" doesn't grab attention",
                &first_line[..first_line.len().min(30)]
            ),
            fix: Some("Lead with a number, question, or bold claim".into()),
        });
        score -= 15;
    }

    let lower = trimmed.to_lowercase();
    let bait_phrases = ["like if", "rt if", "follow for"];
    if bait_phrases.iter().any(|b| lower.contains(b)) {
        issues.push(Issue {
            severity: Severity::Warning,
            code: "engagement_bait".into(),
            message: "Engagement bait detected — X algorithm penalizes this".into(),
            fix: Some("Remove explicit engagement requests".into()),
        });
        score -= 15;
    }

    if features.hashtag_count > 2 {
        issues.push(Issue {
            severity: Severity::Warning,
            code: "excessive_hashtags".into(),
            message: format!(
                "{} hashtags — more than 2 looks spammy and hurts reach",
                features.hashtag_count
            ),
            fix: Some("Keep to 1-2 relevant hashtags max".into()),
        });
        score -= 15;
    }

    if !features.has_numbers && !has_proper_nouns(trimmed) {
        issues.push(Issue {
            severity: Severity::Warning,
            code: "low_specificity".into(),
            message: "No numbers, names, or data — specificity drives engagement".into(),
            fix: Some("Add a concrete number, name, or data point".into()),
        });
        score -= 15;
    }

    if features.char_count < 30 && !features.has_media && !features.has_question {
        // Very short posts with no media or question lack dwell time signal
        issues.push(Issue {
            severity: Severity::Info,
            code: "too_short".into(),
            message: "Very short post — longer content drives more dwell time (a scoring signal)".into(),
            fix: Some("Consider adding depth — the algorithm rewards dwell time".into()),
        });
        score -= 5;
    }

    if trimmed.starts_with('@') {
        issues.push(Issue {
            severity: Severity::Warning,
            code: "starts_with_mention".into(),
            message: "Starting with @mention limits visibility to mutual followers".into(),
            fix: Some("Put a word before the @mention, e.g. \".@user\"".into()),
        });
        score -= 15;
    }

    // --- Info issues (score -= 5) ---
    if !features.has_question {
        issues.push(Issue {
            severity: Severity::Info,
            code: "no_question".into(),
            message: "No question mark — questions drive replies (~20x weight, estimated)".into(),
            fix: Some("Consider ending with a question to invite discussion".into()),
        });
        score -= 5;
    }

    if features.line_count <= 1 && features.char_count > 100 {
        issues.push(Issue {
            severity: Severity::Info,
            code: "no_formatting".into(),
            message: "Wall of text — line breaks improve readability and stop-rate".into(),
            fix: Some("Break into 2-3 short lines".into()),
        });
        score -= 5;
    }

    if trimmed == lower && trimmed.chars().any(|c| c.is_alphabetic()) {
        issues.push(Issue {
            severity: Severity::Info,
            code: "all_lowercase".into(),
            message: "All lowercase — proper capitalization looks more authoritative".into(),
            fix: None,
        });
        score -= 5;
    }

    // --- Positive signals ---
    if features.has_numbers {
        score += 10;
    }
    if features.has_question {
        // Questions always help (~20x weight in algorithm, estimated)
        score += 5;
        if goal == Some("replies") {
            score += 10; // Extra boost when replies is the explicit goal
        }
    }
    if features.char_count > 0 && features.char_count < 200 {
        score += 5;
    }
    if features.line_count > 1 {
        score += 5;
    }
    if features.hook_strength >= 70 {
        score += 10;
    }

    // --- Dwell time estimation ---
    if features.est_dwell_seconds >= 10.0 {
        // Long-form content drives dwell_time signal (~8x) and cont_dwell_time
        score += 5;
    }

    // --- Sentiment check (2026 algorithm: Grok predicts P(block), P(mute)) ---
    if features.sentiment == "negative" {
        issues.push(Issue {
            severity: Severity::Warning,
            code: "negative_sentiment".into(),
            message: "Combative or negative tone — Grok predicts P(block) and P(mute) and suppresses pre-emptively".into(),
            fix: Some("Reframe constructively — critique the idea, not the person".into()),
        });
        score -= 15;
    } else if features.sentiment == "mixed" {
        issues.push(Issue {
            severity: Severity::Info,
            code: "mixed_sentiment".into(),
            message: "Mildly negative language detected — may elevate P(mute) prediction".into(),
            fix: Some("Consider softening — the algorithm penalises predicted negative reactions".into()),
        });
        score -= 5;
    }

    let score = score.clamp(0, 100) as u32;
    let grade = match score {
        90..=100 => "A",
        75..=89 => "B",
        60..=74 => "C",
        40..=59 => "D",
        _ => "F",
    }
    .to_string();

    let suggestions = suggest_improvements(&issues, &features, goal);
    let suggested_next_commands = build_next_commands(trimmed, score);

    let display_text = if trimmed.chars().count() > 200 {
        format!("{}...", crate::utils::safe_truncate(trimmed, 200))
    } else {
        trimmed.to_string()
    };

    PreflightResult {
        text: display_text,
        score,
        grade,
        issues,
        suggestions,
        features,
        suggested_next_commands,
    }
}

fn extract_features(text: &str) -> FeatureVector {
    let char_count = text.chars().count();
    let word_count = text.split_whitespace().count();
    let line_count = text.lines().count();

    let has_link = text.contains("http://") || text.contains("https://");
    let link_position = if has_link { Some("body".into()) } else { None };

    let hashtag_count = text.matches('#').count();
    let has_question = text.contains('?');
    let has_numbers = text.chars().any(|c| c.is_ascii_digit());
    let starts_with_i = text.starts_with("I ") || text.starts_with("I'");

    let cta_patterns = [
        "check out", "click", "sign up", "subscribe", "join", "try it",
        "grab it", "get it", "learn more", "read more", "download",
    ];
    let lower = text.to_lowercase();
    let has_cta = cta_patterns.iter().any(|p| lower.contains(p));

    let hook_strength = score_hook(text.lines().next().unwrap_or(""));
    let content_type_guess = detect_content_type(text);

    // Dwell time: ~200 words per minute reading speed + 1s base scan time
    let est_dwell_seconds = 1.0 + (word_count as f64 / 200.0) * 60.0;

    // Simple sentiment analysis for combative/negative tone
    let negative_words = [
        "stupid", "idiot", "dumb", "hate", "terrible", "awful", "disgusting",
        "pathetic", "garbage", "trash", "worst", "moron", "clown", "fraud",
        "scam", "sucks", "useless", "incompetent", "liar", "bs", "stfu",
        "shut up", "you're wrong", "cope", "ratio",
    ];
    let aggressive_patterns = [
        "imagine thinking", "tell me you", "nobody asked", "stay mad",
        "cry about it", "skill issue", "l + ratio",
    ];
    let neg_count = negative_words.iter().filter(|w| lower.contains(*w)).count();
    let aggro_count = aggressive_patterns.iter().filter(|p| lower.contains(*p)).count();

    let sentiment = if neg_count >= 2 || aggro_count >= 1 {
        "negative".to_string()
    } else if neg_count == 1 {
        "mixed".to_string()
    } else {
        "neutral".to_string()
    };

    FeatureVector {
        char_count,
        word_count,
        has_link,
        link_position,
        has_media: false, // caller can override if media is attached
        hashtag_count,
        has_question,
        has_numbers,
        has_cta,
        hook_strength,
        line_count,
        starts_with_i,
        content_type_guess,
        est_dwell_seconds,
        sentiment,
    }
}

fn score_hook(first_line: &str) -> u32 {
    let trimmed = first_line.trim();
    if trimmed.is_empty() {
        return 0;
    }

    let mut score: u32 = 40; // baseline

    // Starts with a number — strong hook
    if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        score += 30;
    }

    // Starts with a question
    if trimmed.ends_with('?') {
        score += 20;
    }

    // Bold/contrarian signals
    let bold_words = ["never", "always", "stop", "wrong", "truth", "secret", "nobody", "everyone"];
    let lower = trimmed.to_lowercase();
    if bold_words.iter().any(|w| lower.contains(w)) {
        score += 15;
    }

    // Weak openers penalize
    let weak = ["I ", "So ", "Just ", "The ", "It's ", "This is "];
    if weak.iter().any(|w| trimmed.starts_with(w)) {
        score = score.saturating_sub(20);
    }

    score.min(100)
}

fn detect_content_type(text: &str) -> String {
    let lower = text.to_lowercase();

    if lower.contains('?') && lower.lines().count() <= 3 {
        return "question".into();
    }

    let how_to_signals = ["how to", "step 1", "here's how", "guide", "tutorial", "tip:"];
    if how_to_signals.iter().any(|s| lower.contains(s)) {
        return "how-to".into();
    }

    let data_signals = ["%", "million", "billion", "$", "data shows", "study", "research"];
    if data_signals.iter().any(|s| lower.contains(s)) && text.chars().any(|c| c.is_ascii_digit()) {
        return "data".into();
    }

    let announcement_signals = [
        "announcing", "launching", "introducing", "excited to", "just shipped",
        "now available", "new:", "release",
    ];
    if announcement_signals.iter().any(|s| lower.contains(s)) {
        return "announcement".into();
    }

    "opinion".into()
}

fn has_proper_nouns(text: &str) -> bool {
    // Simple heuristic: look for capitalized words that aren't at sentence start
    let words: Vec<&str> = text.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        if i == 0 {
            continue;
        }
        let prev = words[i - 1];
        // Skip words after sentence-ending punctuation
        if prev.ends_with('.') || prev.ends_with('!') || prev.ends_with('?') {
            continue;
        }
        if word.chars().next().is_some_and(|c| c.is_uppercase())
            && !word.starts_with('#')
            && !word.starts_with('@')
            && !word.starts_with("http")
        {
            return true;
        }
    }
    false
}

fn suggest_improvements(issues: &[Issue], features: &FeatureVector, goal: Option<&str>) -> Vec<String> {
    let mut suggestions = Vec::new();

    for issue in issues {
        if let Some(ref fix) = issue.fix {
            suggestions.push(fix.clone());
        }
    }

    // Goal-specific suggestions
    match goal {
        Some("replies") => {
            if !features.has_question {
                suggestions.push("Add a question — questions are the #1 driver of replies (~20x weight)".into());
            }
        }
        Some("impressions") => {
            if features.hook_strength < 70 {
                suggestions.push("Strengthen your hook — first line determines if people stop scrolling".into());
            }
            if features.line_count <= 1 && features.char_count > 80 {
                suggestions.push("Add line breaks — visual spacing increases dwell time (a scoring signal)".into());
            }
        }
        _ => {}
    }

    // Algorithm-smart growth advice (always included)
    if features.est_dwell_seconds < 5.0 && features.char_count > 0 {
        suggestions.push(format!(
            "Est. dwell time: {:.0}s — longer posts drive more dwell_time signal. Consider adding depth.",
            features.est_dwell_seconds
        ));
    }

    if features.content_type_guess == "opinion" && !features.has_numbers {
        suggestions.push("Data-backed opinions outperform pure takes — add a number or citation".into());
    }

    // DM-share potential hint
    if features.content_type_guess == "data" || features.content_type_guess == "how-to" {
        suggestions.push("This looks DM-shareable — insider data and how-tos drive share_via_dm (~25x signal)".into());
    }

    suggestions.dedup();
    suggestions
}

fn build_next_commands(text: &str, score: u32) -> Vec<String> {
    let escaped = text.replace('"', "\\\"");
    if score >= 75 {
        vec![format!("xmaster post \"{}\"", escaped)]
    } else {
        vec![format!("xmaster analyze \"<your revised text>\" --goal replies")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tweet_is_critical() {
        let result = analyze("", None);
        assert!(result.score < 50);
        assert_eq!(result.issues[0].code, "empty_content");
    }

    #[test]
    fn link_in_body_detected() {
        let result = analyze("Check this out https://example.com", None);
        assert!(result.issues.iter().any(|i| i.code == "link_in_body"));
    }

    #[test]
    fn over_280_is_info_long_post() {
        let long = "a".repeat(300);
        let result = analyze(&long, None);
        assert!(result.issues.iter().any(|i| i.code == "long_post"));
    }

    #[test]
    fn over_25000_is_critical() {
        let long = "a".repeat(25_001);
        let result = analyze(&long, None);
        assert!(result.issues.iter().any(|i| i.code == "over_limit"));
    }

    #[test]
    fn clean_tweet_scores_well() {
        let result = analyze("7 things I learned building a startup in 2024:\n\n1. Speed beats perfection\n2. Talk to users daily\n3. Ship or die", None);
        assert!(result.score >= 60, "score was {}", result.score);
        // Content classifier may vary — just check it's a reasonable type
        assert!(!result.features.content_type_guess.is_empty());
    }

    #[test]
    fn question_detected() {
        let result = analyze("What's the hardest lesson you learned this year?", Some("replies"));
        assert!(result.features.has_question);
        // Short questions score lower due to low_specificity — but question is detected
        assert!(result.score >= 50, "score was {}", result.score);
    }

    #[test]
    fn weak_hook_flagged() {
        let result = analyze("I think this is an interesting take on the market", None);
        assert!(result.issues.iter().any(|i| i.code == "weak_hook"));
    }

    #[test]
    fn grade_mapping() {
        let result = analyze("Stop sleeping on Rust.\n\n3 reasons it will dominate backend in 2025:", None);
        assert!(
            ["A", "B", "C"].contains(&result.grade.as_str()),
            "grade was {}",
            result.grade
        );
    }

    #[test]
    fn link_in_body_is_critical() {
        let result = analyze("Great article https://example.com about Rust", None);
        let issue = result.issues.iter().find(|i| i.code == "link_in_body").unwrap();
        assert_eq!(issue.severity, Severity::Critical);
    }

    #[test]
    fn engagement_bait_detected() {
        let result = analyze("Like if you agree with this take on AI", None);
        assert!(result.issues.iter().any(|i| i.code == "engagement_bait"));
    }

    #[test]
    fn starts_with_mention_flagged() {
        let result = analyze("@elonmusk what do you think about this?", None);
        assert!(result.issues.iter().any(|i| i.code == "starts_with_mention"));
    }

    #[test]
    fn at_281_is_long_post_info() {
        let long = "x".repeat(281);
        let result = analyze(&long, None);
        let issue = result.issues.iter().find(|i| i.code == "long_post").unwrap();
        assert_eq!(issue.severity, Severity::Info);
    }

    #[test]
    fn short_question_not_penalized_as_too_short() {
        // Short questions drive replies (~20x estimated weight) — should NOT get "too_short" warning
        let result = analyze("What's your biggest regret?", None);
        assert!(result.features.has_question);
        assert!(
            !result.issues.iter().any(|i| i.code == "too_short"),
            "short question should not be flagged as too_short"
        );
    }

    #[test]
    fn specific_numbers_boost_score() {
        let with_numbers = analyze("3 things I learned building startups in 2024", None);
        let without_numbers = analyze("Things I learned building startups recently", None);
        assert!(
            with_numbers.score > without_numbers.score,
            "with_numbers={} should beat without_numbers={}",
            with_numbers.score,
            without_numbers.score
        );
    }

    #[test]
    fn perfect_tweet_scores_high() {
        // Numbers + question + line breaks + under 200 chars + proper noun
        let text = "3 things Google taught me about scaling:\n\n1. Cache everything\n2. Fail fast\n\nWhat would you add?";
        let result = analyze(text, None);
        assert!(result.score >= 75, "perfect tweet score was {}", result.score);
        assert!(
            result.grade == "A" || result.grade == "B",
            "grade was {}",
            result.grade
        );
    }

    #[test]
    fn empty_text_is_critical() {
        let result = analyze("   ", None);
        let issue = result.issues.iter().find(|i| i.code == "empty_content").unwrap();
        assert_eq!(issue.severity, Severity::Critical);
    }

    #[test]
    fn rt_if_detected_as_engagement_bait() {
        let result = analyze("RT if you think Rust is the future of systems programming", None);
        assert!(result.issues.iter().any(|i| i.code == "engagement_bait"));
    }

    #[test]
    fn excessive_hashtags_warned() {
        let result = analyze("Great day #rust #programming #code #dev", None);
        assert!(result.issues.iter().any(|i| i.code == "excessive_hashtags"));
    }
}
