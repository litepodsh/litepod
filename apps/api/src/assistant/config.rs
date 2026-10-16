//! Which model the assistant talks to, and how.
//!
//! This used to be four environment variables read once at boot. It is now a
//! row in `assistant_connections`, resolved per request, so an organization can
//! bring its own key and change model without a restart.
//!
//! Two things live here and nowhere else: the table of provider kinds the UI
//! offers ([`PROVIDERS`]), and the resolved [`AssistantConfig`] the client layer
//! is handed. Everything below is data — no I/O, no database, no environment.

use serde::{Deserialize, Serialize};

/// How many model calls one turn may make before it is cut off. Each call may
/// run several tools, so this is a ceiling on reasoning depth, not on tool count.
pub const MAX_STEPS: u8 = 6;

/// Anthropic pins its wire format with a dated header rather than a URL version.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// `/messages` requires a limit; `/chat/completions` does not. Generous enough
/// that a turn is never truncated mid-tool-call.
pub const ANTHROPIC_MAX_TOKENS: u32 = 8192;

/// The wire format, which is the only thing that differs between providers once
/// a base URL and a key are known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// `POST {base}/chat/completions`, bearer auth.
    OpenAi,
    /// `POST {base}/messages`, `x-api-key` + `anthropic-version`.
    Anthropic,
}

impl Protocol {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "openai" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            _ => None,
        }
    }
}

/// Whether a provider wants a key at all. Ollama and LM Studio serve a local
/// port and reject nothing; a self-hosted gateway may or may not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum KeyPolicy {
    Required,
    Optional,
    None,
}

/// One kind of provider the "add connection" dialog offers. Serialised straight
/// to the UI so the dialog's grid is not a second hardcoded list in TypeScript.
#[derive(Clone, Copy, Debug, Serialize, utoipa::ToSchema)]
pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    /// Prefilled in the dialog. Empty for the bring-your-own-endpoint kinds.
    pub base_url: &'static str,
    pub protocol: Protocol,
    pub key: KeyPolicy,
    /// True when the user is expected to supply the URL themselves.
    pub custom_url: bool,
    /// OpenCode Zen and Go: they key prompt caching off `x-opencode-session`,
    /// and they route some model ids through endpoints this build does not
    /// speak. Both facts follow from the same flag.
    pub opencode: bool,
}

/// The whole list of provider kinds, and the only place one is defined.
///
/// **Adding a provider is one entry here.** Nothing else: the database column is
/// deliberately unconstrained, every write validates against this list, and the
/// dialog's grid is served from it rather than duplicated in TypeScript. If a
/// new provider needs a second edit anywhere, that is the bug.
///
/// Everything that varies between providers is data in this table, which is why
/// there is no `enum Provider` and no `match` on one. The two things the code
/// genuinely branches on — the wire format and whether a key is required — are
/// the enums above.
pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: "opencode_zen",
        name: "OpenCode Zen",
        base_url: "https://opencode.ai/zen/v1",
        protocol: Protocol::OpenAi,
        key: KeyPolicy::Required,
        custom_url: false,
        opencode: true,
    },
    Provider {
        id: "opencode_go",
        name: "OpenCode Go",
        base_url: "https://opencode.ai/zen/go/v1",
        protocol: Protocol::OpenAi,
        key: KeyPolicy::Required,
        custom_url: false,
        opencode: true,
    },
    Provider {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        protocol: Protocol::OpenAi,
        key: KeyPolicy::Required,
        custom_url: false,
        opencode: false,
    },
    Provider {
        id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        protocol: Protocol::OpenAi,
        key: KeyPolicy::Required,
        custom_url: false,
        opencode: false,
    },
    Provider {
        id: "anthropic",
        name: "Anthropic",
        base_url: "https://api.anthropic.com/v1",
        protocol: Protocol::Anthropic,
        key: KeyPolicy::Required,
        custom_url: false,
        opencode: false,
    },
    Provider {
        id: "ollama",
        name: "Ollama",
        base_url: "http://127.0.0.1:11434/v1",
        protocol: Protocol::OpenAi,
        key: KeyPolicy::None,
        custom_url: true,
        opencode: false,
    },
    Provider {
        id: "lmstudio",
        name: "LM Studio",
        base_url: "http://127.0.0.1:1234/v1",
        protocol: Protocol::OpenAi,
        key: KeyPolicy::None,
        custom_url: true,
        opencode: false,
    },
    Provider {
        id: "openai_compatible",
        name: "OpenAI-compatible",
        base_url: "",
        protocol: Protocol::OpenAi,
        key: KeyPolicy::Optional,
        custom_url: true,
        opencode: false,
    },
    Provider {
        id: "anthropic_compatible",
        name: "Claude-compatible",
        base_url: "",
        protocol: Protocol::Anthropic,
        key: KeyPolicy::Optional,
        custom_url: true,
        opencode: false,
    },
];

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|provider| provider.id == id)
}

impl Provider {
    /// Whether a connection of this kind may dial a private or loopback address.
    ///
    /// Only the keyless kinds (Ollama, LM Studio) serve a local port, and only a
    /// self-hosted install has a local network that belongs to the customer. On
    /// cloud the "local network" is the control plane's own — the unauthenticated
    /// Caddy admin API, the VPS metadata service — so it is never reachable.
    pub fn allows_local(&self, is_cloud: bool) -> bool {
        !is_cloud && self.key == KeyPolicy::None
    }

    /// Whether this kind can be used at all on this edition. The keyless kinds
    /// exist only to reach a local model server, which cloud refuses.
    pub fn available(&self, is_cloud: bool) -> bool {
        !is_cloud || self.key != KeyPolicy::None
    }
}

/// The provider kinds this edition offers.
pub fn offered(is_cloud: bool) -> Vec<Provider> {
    PROVIDERS
        .iter()
        .filter(|provider| provider.available(is_cloud))
        .copied()
        .collect()
}

/// One connection, decrypted, ready to call. Built per request from a row; never
/// cached, so revoking a key takes effect on the next message.
#[derive(Clone, Debug)]
pub struct AssistantConfig {
    /// `None` for a local endpoint that authenticates nothing.
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub protocol: Protocol,
    pub opencode: bool,
    /// Whether this connection may point at a private address. See
    /// [`Provider::allows_local`] and [`crate::assistant::endpoint`].
    pub allow_local: bool,
}

impl AssistantConfig {
    /// Both protocols list models at the same path; only the auth header differs.
    pub fn models_url(&self) -> String {
        format!("{}/models", self.base_url)
    }
}

/// The upstream endpoint family a model id needs. OpenCode routes some of its
/// own ids through `/responses` or `/messages` instead of `/chat/completions`,
/// and the rest of the code branches on this — at call time to pick a path,
/// and when a catalog is fetched to mark an unrunnable id as listed-but-blocked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointFamily {
    /// `POST {base}/chat/completions` with bearer auth. The default.
    ChatCompletions,
    /// `POST {base}/messages` with `x-api-key` + `anthropic-version`.
    Messages,
    /// `POST {base}/responses`. Not spoken by this build; the catalog marks
    /// these models as listed-but-blocked rather than letting a turn fail with
    /// a 404 the user cannot act on.
    Responses,
    /// Served through a route this build does not speak at all. Catalog-only.
    Unsupported,
}

impl EndpointFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat/completions",
            Self::Messages => "messages",
            Self::Responses => "responses",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Routes an OpenCode model id to the endpoint family OpenCode actually serves
/// it through. Non-OpenCode connections always land on `/chat/completions` —
/// the prefix table only matters when the gateway splits one provider across
/// three wire shapes.
///
/// Both OpenCode tiers prefix ids (`opencode/…`, `opencode-go/…`); the prefix
/// is stripped before matching so the table stays flat.
pub fn endpoint_family_for(opencode: bool, model: &str) -> EndpointFamily {
    if !opencode {
        return EndpointFamily::ChatCompletions;
    }
    let bare = model.rsplit('/').next().unwrap_or(model);
    if bare.starts_with("claude-") || bare.starts_with("minimax-") || bare.starts_with("qwen") {
        EndpointFamily::Messages
    } else if bare.starts_with("gpt-") || bare.starts_with("grok-") || bare.starts_with("muse-") {
        EndpointFamily::Responses
    } else if bare.starts_with("gemini-") {
        EndpointFamily::Unsupported
    } else {
        // `glm-`, `kimi-`, `longcat-`, `deepseek-`, `mimo-`, `hy`, `omen-`, …
        EndpointFamily::ChatCompletions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_family_matches_the_route_opencode_actually_serves() {
        // Anthropic Messages — spoken by this build.
        assert_eq!(
            endpoint_family_for(true, "opencode/claude-opus-4-5"),
            EndpointFamily::Messages
        );
        assert_eq!(
            endpoint_family_for(true, "opencode-go/minimax-m3"),
            EndpointFamily::Messages
        );
        assert_eq!(
            endpoint_family_for(true, "opencode-go/qwen3.8-max"),
            EndpointFamily::Messages
        );
        // Chat Completions — default for everything else on opencode.
        assert_eq!(
            endpoint_family_for(true, "opencode-go/kimi-k2.6"),
            EndpointFamily::ChatCompletions
        );
        assert_eq!(
            endpoint_family_for(true, "opencode-go/deepseek-v4-flash"),
            EndpointFamily::ChatCompletions
        );
        // Responses — not spoken; the catalog will block these.
        assert_eq!(
            endpoint_family_for(true, "opencode-go/grok-4.6"),
            EndpointFamily::Responses
        );
        assert_eq!(
            endpoint_family_for(true, "opencode-go/muse-spark-1.3-contributor"),
            EndpointFamily::Responses
        );
        // Gemini — unsupported outright.
        assert_eq!(
            endpoint_family_for(true, "opencode-go/gemini-2.0-flash"),
            EndpointFamily::Unsupported
        );
        // Non-opencode always lands on chat/completions, regardless of prefix.
        assert_eq!(
            endpoint_family_for(false, "claude-opus-4-5"),
            EndpointFamily::ChatCompletions
        );
        assert_eq!(
            endpoint_family_for(false, "gpt-5.2"),
            EndpointFamily::ChatCompletions
        );
    }

    #[test]
    fn every_provider_id_is_unique_and_resolvable() {
        for entry in PROVIDERS {
            assert_eq!(provider(entry.id).map(|p| p.id), Some(entry.id));
        }
        let mut ids: Vec<&str> = PROVIDERS.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate provider id");
    }

    #[test]
    fn a_hosted_provider_ships_a_base_url_and_a_custom_one_does_not() {
        assert_eq!(
            provider("anthropic").unwrap().base_url,
            "https://api.anthropic.com/v1"
        );
        assert!(provider("openai_compatible").unwrap().base_url.is_empty());
        assert!(provider("openai_compatible").unwrap().custom_url);
    }

    #[test]
    fn urls_hang_off_the_base() {
        let config = AssistantConfig {
            api_key: None,
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "llama3.2:3b".into(),
            protocol: Protocol::OpenAi,
            opencode: false,
            allow_local: true,
        };
        assert_eq!(config.models_url(), "http://127.0.0.1:11434/v1/models");
    }

    #[test]
    fn cloud_offers_no_provider_that_may_reach_a_private_address() {
        for entry in PROVIDERS {
            assert!(
                !entry.allows_local(true),
                "{} reaches local on cloud",
                entry.id
            );
        }
        let offered_ids: Vec<&str> = offered(true).iter().map(|p| p.id).collect();
        assert!(!offered_ids.contains(&"ollama") && !offered_ids.contains(&"lmstudio"));
        // Self-hosted keeps the local model servers it always had.
        assert!(provider("ollama").unwrap().allows_local(false));
        assert!(!provider("openai_compatible").unwrap().allows_local(false));
        assert_eq!(offered(false).len(), PROVIDERS.len());
    }
}
