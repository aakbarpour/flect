//! Versioned cost estimates for known `OpenAI` model identifiers.

use crate::RunnerMetadata;

/// Pricing snapshot used by Flect's `OpenAI` cost estimates.
pub const OPENAI_PRICING_VERSION: &str = "openai-2026-08-12";

struct ModelPrice {
    model: &'static str,
    input_per_million: f64,
    cached_input_per_million: f64,
    output_per_million: f64,
}

const PRICING_TABLE: &[ModelPrice] = &[
    ModelPrice {
        model: "gpt-5.6-luna",
        input_per_million: 1.0,
        cached_input_per_million: 0.1,
        output_per_million: 6.0,
    },
    ModelPrice {
        model: "gpt-5.6-terra",
        input_per_million: 2.5,
        cached_input_per_million: 0.25,
        output_per_million: 15.0,
    },
];

/// Estimates cost only for the official `OpenAI` endpoint and known pricing.
pub fn estimate_openai_cost(metadata: &RunnerMetadata, base_url: &str) -> Option<f64> {
    if !base_url
        .trim_end_matches('/')
        .eq_ignore_ascii_case("https://api.openai.com/v1")
    {
        return None;
    }
    let price = PRICING_TABLE
        .iter()
        .find(|price| price.model == metadata.model)?;
    let input = metadata.usage.input_tokens?;
    let cached = metadata.usage.cached_input_tokens.unwrap_or(0).min(input);
    let output = metadata.usage.output_tokens?;
    let uncached = input.saturating_sub(cached);
    let long_context = input > 272_000;
    let uncached = f64::from(u32::try_from(uncached).ok()?);
    let cached = f64::from(u32::try_from(cached).ok()?);
    let output = f64::from(u32::try_from(output).ok()?);
    Some(
        ((uncached * price.input_per_million * if long_context { 2.0 } else { 1.0 })
            + (cached * price.cached_input_per_million * if long_context { 2.0 } else { 1.0 })
            + (output * price.output_per_million * if long_context { 1.5 } else { 1.0 }))
            / 1_000_000.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TokenUsage;

    #[test]
    fn estimates_only_known_models_on_the_official_endpoint() {
        let metadata = RunnerMetadata {
            provider: "openai-compatible".to_owned(),
            model: "gpt-5.6-luna".to_owned(),
            latency_ms: 1,
            usage: TokenUsage {
                input_tokens: Some(100_000),
                cached_input_tokens: Some(50_000),
                output_tokens: Some(10_000),
            },
        };
        assert_eq!(
            estimate_openai_cost(&metadata, "https://api.openai.com/v1"),
            Some(0.115)
        );
        assert_eq!(
            estimate_openai_cost(&metadata, "https://example.com/v1"),
            None
        );
    }
}
