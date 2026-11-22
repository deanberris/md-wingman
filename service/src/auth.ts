// Which credential, and which model server, Claude Code uses for the
// service's requests.
//
// - "subscription" (the default): Claude Code uses the login from `claude`
//   (or a CLAUDE_CODE_OAUTH_TOKEN from `claude setup-token`), so usage counts
//   against the user's Claude plan. API keys are removed from the environment,
//   so a key that happens to be set can't quietly switch billing to the API.
// - "api_key": Claude Code uses ANTHROPIC_API_KEY and bills the Claude API.
//   Subscription tokens are removed so the key is what gets used.
// - "provider": Claude Code talks to another Anthropic-compatible endpoint,
//   such as OpenRouter or a local model server (Ollama, LM Studio). See
//   `providerFromEnv` for the variables it reads.
//
// Choose with WINGMAN_AUTH=subscription, api_key or provider.

export type AuthMode = "subscription" | "api_key" | "provider";

/** Variables that make Claude Code bill an API account instead of the plan. */
const API_CREDENTIAL_VARS = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_FOUNDRY_API_KEY"];

/** Variables that carry a subscription login. */
const SUBSCRIPTION_CREDENTIAL_VARS = ["CLAUDE_CODE_OAUTH_TOKEN"];

/** Variables that send Claude Code to Bedrock, Vertex or Foundry instead. */
const CLOUD_PROVIDER_VARS = ["CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY"];

/**
 * Sent as the token when a provider needs none (most local servers). Claude
 * Code must always have one: without it, it would fall back to the user's
 * Claude login and send that to the provider's URL.
 */
export const NO_TOKEN = "md-wingman-no-token";

/** Another Anthropic-compatible endpoint for Claude Code to use. */
export interface Provider {
  /** Shown to the writer, such as "OpenRouter". */
  name: string;
  /** The endpoint's base URL, without `/v1`, such as https://openrouter.ai/api. */
  baseUrl: string;
  /** The API key or token, if the provider needs one. */
  token?: string;
  /** The model for the tasks, in the provider's own naming. */
  model: string;
  /** The model for Claude Code's quick background calls; `model` if unset. */
  fastModel?: string;
  /** Whether the endpoint runs Anthropic's server-side web search tool. */
  webSearch: boolean;
}

export function authModeFromEnv(env: NodeJS.ProcessEnv = process.env): AuthMode {
  const value = (env.WINGMAN_AUTH ?? "subscription").trim().toLowerCase().replace("-", "_");
  if (value === "subscription" || value === "api_key" || value === "provider") return value;
  throw new Error(`WINGMAN_AUTH must be "subscription", "api_key" or "provider", not "${env.WINGMAN_AUTH}"`);
}

/**
 * Reads the provider for "provider" mode:
 *
 * - WINGMAN_PROVIDER_URL: the base URL (required)
 * - WINGMAN_MODEL: the model (required)
 * - WINGMAN_FAST_MODEL: the model for background calls (optional)
 * - WINGMAN_PROVIDER_TOKEN: the API key or token (optional for local servers)
 * - WINGMAN_PROVIDER_NAME: a name to show (optional)
 * - WINGMAN_WEB_SEARCH=0: the endpoint has no web search tool
 */
export function providerFromEnv(env: NodeJS.ProcessEnv = process.env): Provider {
  const baseUrl = env.WINGMAN_PROVIDER_URL?.trim() ?? "";
  if (!baseUrl) throw new Error("WINGMAN_AUTH=provider needs WINGMAN_PROVIDER_URL, the provider's base URL");
  let url: URL;
  try {
    url = new URL(baseUrl);
  } catch {
    throw new Error(`WINGMAN_PROVIDER_URL isn't a URL: ${baseUrl}`);
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") {
    throw new Error(`WINGMAN_PROVIDER_URL must be an http or https URL, not ${baseUrl}`);
  }
  const model = env.WINGMAN_MODEL?.trim() ?? "";
  if (!model) throw new Error("WINGMAN_AUTH=provider needs WINGMAN_MODEL, the provider's name for the model");
  return {
    name: env.WINGMAN_PROVIDER_NAME?.trim() || url.host,
    baseUrl: baseUrl.replace(/\/+$/, ""),
    token: env.WINGMAN_PROVIDER_TOKEN?.trim() || undefined,
    model,
    fastModel: env.WINGMAN_FAST_MODEL?.trim() || undefined,
    webSearch: env.WINGMAN_WEB_SEARCH?.trim() !== "0",
  };
}

/** The environment to start Claude Code with, for the given mode. */
export function claudeEnv(
  mode: AuthMode,
  env: NodeJS.ProcessEnv = process.env,
  provider?: Provider,
): Record<string, string | undefined> {
  const out: Record<string, string | undefined> = { ...env };
  // The app's own settings for the provider aren't for Claude Code.
  delete out.WINGMAN_PROVIDER_TOKEN;
  if (mode === "provider") {
    if (!provider) throw new Error("provider mode needs a provider");
    for (const name of [...API_CREDENTIAL_VARS, ...SUBSCRIPTION_CREDENTIAL_VARS, ...CLOUD_PROVIDER_VARS]) {
      delete out[name];
    }
    const fast = provider.fastModel ?? provider.model;
    Object.assign(out, {
      ANTHROPIC_BASE_URL: provider.baseUrl,
      // Sent as a bearer token, which is what OpenRouter and local servers
      // expect. The API key must be present and empty, or Claude Code may
      // go back to the Claude login.
      ANTHROPIC_AUTH_TOKEN: provider.token ?? NO_TOKEN,
      ANTHROPIC_API_KEY: "",
      ANTHROPIC_MODEL: provider.model,
      ANTHROPIC_DEFAULT_OPUS_MODEL: provider.model,
      ANTHROPIC_DEFAULT_SONNET_MODEL: provider.model,
      ANTHROPIC_DEFAULT_HAIKU_MODEL: fast,
      CLAUDE_CODE_SUBAGENT_MODEL: provider.model,
    });
    return out;
  }
  const remove = mode === "subscription" ? API_CREDENTIAL_VARS : SUBSCRIPTION_CREDENTIAL_VARS;
  for (const name of remove) delete out[name];
  if (mode === "api_key" && !out.ANTHROPIC_API_KEY) {
    throw new Error("WINGMAN_AUTH=api_key needs ANTHROPIC_API_KEY to be set");
  }
  return out;
}
